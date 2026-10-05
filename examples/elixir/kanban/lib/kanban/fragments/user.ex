defmodule Kanban.Fragments.User do
  @moduledoc """
  What a user is, whatever stores them. Each of the kanban resources is a fragment like this
  one, which `Kanban.Memory` (ETS) and `Kanban.Sqlite` (SQLite) each complete with a data
  layer and the relationships, which name resources of their own.
  """
  use Spark.Dsl.Fragment, of: Ash.Resource, authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :name, :string, allow_nil?: false, public?: true
    attribute :email, :string, allow_nil?: false, public?: true
  end

  actions do
    defaults [:destroy]

    create :register do
      primary? true
      accept [:name, :email]
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
