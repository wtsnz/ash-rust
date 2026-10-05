defmodule Helpdesk.TicketFragment do
  @moduledoc """
  What a ticket is, whatever stores it: `Helpdesk.Memory.Ticket` keeps it in ETS and
  `Helpdesk.Sqlite.Ticket` in SQLite, as the Rust `Ticket` runs on either data layer.
  Each adds its data layer and its relationship to the representative.
  """
  use Spark.Dsl.Fragment, of: Ash.Resource, authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :subject, :string, allow_nil?: false, public?: true

    attribute :status, :atom,
      allow_nil?: false,
      public?: true,
      constraints: [one_of: [:open, :closed]]

    attribute :opener_id, :uuid, allow_nil?: false, public?: true
    attribute :estimate, :float, public?: true
    attribute :due_on, :date, public?: true
    attribute :attachment, :binary, public?: true
    attribute :requester_email, :ci_string, public?: true
  end

  calculations do
    calculate :subject_length, :integer, expr(string_length(subject)), public?: true
  end

  actions do
    defaults [:destroy]

    create :open do
      accept [:subject, :estimate, :due_on, :attachment, :requester_email]
      validate string_length(:subject, min: 2)
      change set_attribute(:status, :open)
      change Helpdesk.Changes.RelateActor
    end

    read :read do
      primary? true
      pagination keyset?: true, offset?: true, countable: true, required?: false
    end

    update :assign do
      accept [:representative_id]
    end

    update :close do
      require_atomic? false
      change set_attribute(:status, :closed)
    end

    action :analyze_subject, :map do
      argument :text, :string, allow_nil?: false

      constraints fields: [
                    word_count: [type: :integer, allow_nil?: false],
                    urgent: [type: :boolean, allow_nil?: false]
                  ]

      run fn input, _context ->
        text = input.arguments.text

        {:ok,
         %{
           word_count: text |> String.split() |> length(),
           urgent: String.contains?(text, "!")
         }}
      end
    end

    create :intake do
      accept [:subject]
      manual Helpdesk.Intake
      change set_attribute(:status, :open)
      change Helpdesk.Changes.RelateActor
    end
  end

  policies do
    policy action([:open, :analyze_subject, :intake]) do
      authorize_if actor_present()
    end

    policy action_type(:read) do
      authorize_if expr(opener_id == ^actor(:id))
      authorize_if expr(representative_id == ^actor(:id))
      authorize_if expr(is_nil(representative_id) and ^actor(:role) == "representative")
    end

    policy action(:assign) do
      authorize_if actor_attribute_equals(:role, "representative")
    end

    policy action(:close) do
      authorize_if expr(opener_id == ^actor(:id))
      authorize_if expr(representative_id == ^actor(:id))
    end
  end
end
