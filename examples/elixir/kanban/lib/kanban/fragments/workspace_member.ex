defmodule Kanban.Fragments.WorkspaceMember do
  @moduledoc "A user's place in a workspace. See `Kanban.Fragments.User`."
  use Spark.Dsl.Fragment, of: Ash.Resource, authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :workspace_id, :uuid, allow_nil?: false, public?: true
    attribute :user_id, :uuid, allow_nil?: false, public?: true

    attribute :role, :atom,
      allow_nil?: false,
      public?: true,
      constraints: [one_of: [:admin, :member, :guest]]
  end

  actions do
    defaults [:destroy]

    create :add do
      primary? true
      accept [:workspace_id, :user_id, :role]
    end

    read :read do
      primary? true
      pagination keyset?: true, offset?: true, countable: true, required?: false
    end
  end

  policies do
    policy always() do
      authorize_if always()
    end
  end
end
