defmodule Kanban.Fragments.Card do
  @moduledoc "What a card is: a task on a list. See `Kanban.Fragments.User`."
  use Spark.Dsl.Fragment, of: Ash.Resource, authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :board_id, :uuid, allow_nil?: false, public?: true
    attribute :list_id, :uuid, allow_nil?: false, public?: true
    attribute :title, :string, allow_nil?: false, public?: true
    attribute :description, :string, public?: true
    attribute :position, :integer, allow_nil?: false, public?: true
    attribute :estimate, :float, public?: true
    attribute :due_on, :date, public?: true
    attribute :attachment, :binary, public?: true
    attribute :requester_email, :ci_string, public?: true
    attribute :archived, :boolean, allow_nil?: false, default: false, public?: true
    attribute :creator_id, :uuid, public?: true
    attribute :assignee_id, :uuid, public?: true
  end

  calculations do
    calculate :title_length, :integer, expr(string_length(title)), public?: true
  end

  actions do
    defaults [:destroy]

    create :create do
      primary? true

      accept [
        :board_id,
        :list_id,
        :title,
        :description,
        :position,
        :estimate,
        :due_on,
        :attachment,
        :requester_email
      ]

      validate string_length(:title, min: 1)
      change set_attribute(:archived, false)
      change {Kanban.Changes.RelateActor, attribute: :creator_id}
    end

    read :read do
      primary? true
      pagination keyset?: true, offset?: true, countable: true, required?: false
    end

    update :update_details do
      accept [:title, :description]
    end

    update :move_to_list do
      accept [:list_id, :position]
    end

    update :assign do
      accept [:assignee_id]
    end

    update :archive do
      change set_attribute(:archived, true)
    end
  end

  policies do
    policy always() do
      authorize_if always()
    end
  end
end
