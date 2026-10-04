defmodule Supportdesk.Desk.Ticket do
  @moduledoc "A customer's request, and the desk's work on it."
  use Ash.Resource,
    domain: Supportdesk.Desk,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshStateMachine, AshGraphql.Resource, AshTypescript.Resource],
    authorizers: [Ash.Policy.Authorizer],
    notifiers: [Ash.Notifier.PubSub]

  require Ash.Query

  alias Supportdesk.Desk.{Agent, AuditEvent}

  postgres do
    table "tickets"
    repo Supportdesk.Repo
  end

  multitenancy do
    strategy :attribute
    attribute :org
  end

  state_machine do
    state_attribute :status
    # A ticket opens as new; the fixture loads tickets in every state.
    initial_states [:new, :open, :pending, :resolved, :closed]
    default_initial_state :new

    transitions do
      transition :start, from: :new, to: :open
      transition :hold, from: :open, to: :pending
      transition :resolve, from: [:open, :pending], to: :resolved
      transition :reopen, from: :resolved, to: :open
      transition :close, from: :resolved, to: :closed
    end
  end

  attributes do
    uuid_primary_key :id, writable?: true
    attribute :org, :string, allow_nil?: false, public?: true
    attribute :subject, :string, allow_nil?: false, public?: true
    attribute :body, :string, allow_nil?: false, public?: true
    # 1 (low) to 4 (urgent).
    attribute :priority, :integer, allow_nil?: false, public?: true
    # Hidden from viewers.
    attribute :confidential, :boolean, allow_nil?: false, default: false, public?: true
    # Only an admin and the ticket's assignee see it. Nullable, as in the Rust desk, where
    # a redacted read holds null; `open` requires it.
    attribute :requester_email, :string, public?: true

    attribute :status, :atom,
      allow_nil?: false,
      default: :new,
      public?: true,
      constraints: [one_of: [:new, :open, :pending, :resolved, :closed]]

    attribute :view_count, :integer, allow_nil?: false, default: 0, public?: true
    attribute :reopen_count, :integer, allow_nil?: false, default: 0, public?: true
    attribute :version, :integer, allow_nil?: false, default: 1, public?: true
    create_timestamp :inserted_at, public?: true, writable?: true
    update_timestamp :updated_at, public?: true, writable?: true
  end

  relationships do
    belongs_to :assignee, Agent, public?: true, attribute_writable?: true
    belongs_to :author, Agent, public?: true, attribute_writable?: true
    has_many :comments, Supportdesk.Desk.Comment, public?: true

    many_to_many :tags, Supportdesk.Desk.Tag do
      through Supportdesk.Desk.TicketTag
      source_attribute_on_join_resource :ticket_id
      destination_attribute_on_join_resource :tag_id
      public? true
    end
  end

  aggregates do
    count :comment_count, :comments, public?: true

    count :public_comment_count, :comments do
      filter expr(internal == false)
      public? true
    end

    exists :has_internal_notes, :comments do
      filter expr(internal == true)
      public? true
    end
  end

  calculations do
    calculate :weight, :integer, expr(priority * 10), public?: true
    calculate :subject_length, :integer, expr(string_length(subject)), public?: true

    calculate :scaled_priority, :integer, expr(priority * ^arg(:factor)) do
      public? true
      argument :factor, :integer, allow_nil?: false
    end
  end

  policies do
    bypass actor_attribute_equals(:role, "admin") do
      authorize_if always()
    end

    policy action_type(:read) do
      authorize_if actor_attribute_equals(:role, "agent")
      authorize_if expr(confidential == false)
    end

    policy action_type([:create, :update, :destroy, :action]) do
      authorize_if actor_attribute_equals(:role, "agent")
    end
  end

  field_policies do
    field_policy :requester_email do
      authorize_if expr(assignee_id == ^actor(:id))
      authorize_if actor_attribute_equals(:role, "admin")
    end

    field_policy :* do
      authorize_if always()
    end
  end

  changes do
    # Every update moves the lock version, as the Rust desk's `[version]` attribute does.
    change optimistic_lock(:version), on: [:update]
  end

  actions do
    read :read do
      primary? true
      pagination keyset?: true, countable: true, required?: false
    end

    create :open do
      primary? true
      accept [:subject, :body, :priority, :confidential, :requester_email]
      argument :comments, {:array, :map}, default: []
      validate present(:requester_email)
      validate string_length(:subject, min: 3, max: 200)
      validate numericality(:priority, greater_than_or_equal_to: 1, less_than_or_equal_to: 4)
      change set_attribute(:author_id, actor(:id))
      change manage_relationship(:comments, type: :create)
    end

    update :assign do
      accept [:assignee_id]
    end

    update :start do
      change transition_state(:open)
    end

    update :hold do
      change transition_state(:pending)
    end

    update :resolve do
      change transition_state(:resolved)
    end

    update :reopen do
      change transition_state(:open)
      change atomic_update(:reopen_count, expr(reopen_count + 1))
    end

    update :close do
      change transition_state(:closed)
    end

    update :view do
      change atomic_update(:view_count, expr(view_count + 1))
    end

    update :edit do
      accept [:subject, :priority]
      validate string_length(:subject, min: 3, max: 200)
      validate numericality(:priority, greater_than_or_equal_to: 1, less_than_or_equal_to: 4)
    end

    # Opens a ticket with its comments, assigns it to the active agent or admin with the
    # fewest open tickets (then by name), and records it, in one transaction.
    action :route, :uuid do
      transaction? true
      argument :subject, :string, allow_nil?: false
      argument :body, :string, allow_nil?: false
      argument :priority, :integer, allow_nil?: false
      argument :requester_email, :string, allow_nil?: false
      argument :comments, {:array, :map}, default: []

      run fn input, context ->
        opts = Ash.Context.to_opts(context)
        args = input.arguments

        with {:ok, ticket} <-
               __MODULE__
               |> Ash.Changeset.for_create(:open, Map.take(args, [:subject, :body, :priority, :requester_email, :comments]), opts)
               |> Ash.create(),
             {:ok, agent} <-
               Agent
               |> Ash.Query.filter(active == true and role in ["agent", "admin"])
               |> Ash.Query.load(:open_assigned)
               |> Ash.Query.sort(open_assigned: :asc, name: :asc)
               |> Ash.Query.limit(1)
               |> Ash.read_one(opts),
             {:ok, ticket} <- assign(ticket, agent, opts),
             {:ok, _event} <-
               AuditEvent
               |> Ash.Changeset.for_create(:record, %{ticket_id: ticket.id, kind: "routed"}, opts)
               |> Ash.create() do
          {:ok, ticket.id}
        end
      end
    end

    destroy :destroy do
      primary? true
    end

    create :seed do
      accept [
        :id,
        :org,
        :subject,
        :body,
        :priority,
        :confidential,
        :requester_email,
        :assignee_id,
        :author_id,
        :status,
        :view_count,
        :reopen_count,
        :version,
        :inserted_at,
        :updated_at
      ]
    end
  end

  defp assign(ticket, nil, _opts), do: {:ok, ticket}

  defp assign(ticket, agent, opts) do
    ticket |> Ash.Changeset.for_update(:assign, %{assignee_id: agent.id}, opts) |> Ash.update()
  end

  typescript do
    type_name "Ticket"
  end

  graphql do
    type :ticket

    queries do
      get :get_ticket, :read
      list :list_tickets, :read
    end

    mutations do
      create :open_ticket, :open
      update :assign_ticket, :assign
      update :start_ticket, :start
      update :hold_ticket, :hold
      update :resolve_ticket, :resolve
      update :reopen_ticket, :reopen
      update :close_ticket, :close
      update :view_ticket, :view
      update :edit_ticket, :edit
      destroy :destroy_ticket, :destroy
      action :route_ticket, :route
    end

    subscriptions do
      pubsub SupportdeskWeb.Endpoint

      subscribe :ticket_created do
        action_types [:create]
      end

      subscribe :ticket_updated do
        action_types [:update]
      end

      subscribe :ticket_destroyed do
        action_types [:destroy]
      end
    end
  end
end
