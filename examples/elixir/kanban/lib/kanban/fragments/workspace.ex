defmodule Kanban.Fragments.Workspace do
  @moduledoc "What a workspace is: a team's home for boards. See `Kanban.Fragments.User`."
  use Spark.Dsl.Fragment, of: Ash.Resource, authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :name, :string, allow_nil?: false, public?: true
    attribute :slug, :string, allow_nil?: false, public?: true
    attribute :owner_id, :uuid, allow_nil?: false, public?: true
  end

  actions do
    defaults [:destroy]

    create :create do
      primary? true
      accept [:name, :slug]
      change {Kanban.Changes.RelateActor, attribute: :owner_id}
    end

    read :read do
      primary? true
      pagination keyset?: true, offset?: true, countable: true, required?: false
    end
  end

  policies do
    policy action(:create) do
      authorize_if actor_present()
    end

    policy action_type(:read) do
      authorize_if always()
    end
  end
end
