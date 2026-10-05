defmodule Kanban.Fragments.Board do
  @moduledoc "What a board is: lists of cards for a workspace. See `Kanban.Fragments.User`."
  use Spark.Dsl.Fragment, of: Ash.Resource, authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :workspace_id, :uuid, allow_nil?: false, public?: true
    attribute :name, :string, allow_nil?: false, public?: true
    attribute :description, :string, public?: true
    attribute :archived, :boolean, allow_nil?: false, default: false, public?: true
  end

  actions do
    defaults [:destroy]

    create :create do
      primary? true
      accept [:workspace_id, :name, :description]
      change set_attribute(:archived, false)
    end

    read :read do
      primary? true
      pagination keyset?: true, offset?: true, countable: true, required?: false
    end

    update :update_details do
      accept [:name, :description]
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
