defmodule AstroHelpdesk.Desk.Ticket do
  @moduledoc "A ticket on the desk."
  use Ash.Resource,
    domain: AstroHelpdesk.Desk,
    data_layer: Ash.DataLayer.Ets,
    extensions: [AshGraphql.Resource],
    notifiers: [Ash.Notifier.PubSub]

  attributes do
    uuid_primary_key :id
    attribute :title, :string, allow_nil?: false, public?: true
    attribute :description, :string, public?: true
    attribute :status, AstroHelpdesk.Types.TicketStatus, allow_nil?: false, public?: true
    attribute :priority, :integer, allow_nil?: false, public?: true
    attribute :estimate, :float, public?: true
    attribute :due_on, :date, public?: true
    # Bytes can't be filtered on, as the Rust desk's `Binary` can't.
    attribute :attachment, AstroHelpdesk.Types.Attachment, public?: true, filterable?: false
    attribute :requester_email, :ci_string, public?: true
  end

  relationships do
    belongs_to :author, AstroHelpdesk.Desk.Representative,
      public?: true,
      attribute_writable?: true
  end

  actions do
    defaults [:read]

    create :open do
      primary? true

      accept [
        :title,
        :description,
        :status,
        :priority,
        :author_id,
        :estimate,
        :due_on,
        :attachment,
        :requester_email
      ]

      validate string_length(:title, min: 5, max: 100)
      validate numericality(:priority, greater_than_or_equal_to: 1, less_than_or_equal_to: 5)
    end

    update :change_status do
      primary? true
      accept [:status]
    end

    update :update_details do
      accept [:title, :description, :priority]
      validate string_length(:title, min: 5, max: 100)
      validate numericality(:priority, greater_than_or_equal_to: 1, less_than_or_equal_to: 5)
    end

    destroy :close do
      primary? true
    end
  end

  graphql do
    type :ticket

    queries do
      get :get_ticket, :read
      list :list_tickets, :read, paginate_with: :keyset
    end

    mutations do
      create :open_ticket, :open
      update :change_status_ticket, :change_status
      update :update_details_ticket, :update_details
      destroy :close_ticket, :close
    end

    subscriptions do
      pubsub AstroHelpdeskWeb.Endpoint

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
