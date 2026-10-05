defmodule Kanban.Fragments.ChecklistItem do
  @moduledoc "What a checklist item is: a subtask on a card. See `Kanban.Fragments.User`."
  use Spark.Dsl.Fragment, of: Ash.Resource, authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :card_id, :uuid, allow_nil?: false, public?: true
    attribute :title, :string, allow_nil?: false, public?: true
    attribute :completed, :boolean, allow_nil?: false, default: false, public?: true
    attribute :position, :integer, allow_nil?: false, public?: true
  end

  actions do
    defaults [:destroy]

    create :create do
      primary? true
      accept [:card_id, :title, :position]
      change set_attribute(:completed, false)
    end

    read :read do
      primary? true
      pagination keyset?: true, offset?: true, countable: true, required?: false
    end

    update :toggle do
      accept [:completed]
    end
  end

  policies do
    policy always() do
      authorize_if always()
    end
  end
end
