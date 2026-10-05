defmodule Kanban.Fragments.Comment do
  @moduledoc "What a comment is: discussion on a card. See `Kanban.Fragments.User`."
  use Spark.Dsl.Fragment, of: Ash.Resource, authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :card_id, :uuid, allow_nil?: false, public?: true
    attribute :author_id, :uuid, public?: true
    attribute :body, :string, allow_nil?: false, public?: true
  end

  actions do
    defaults [:destroy]

    create :create do
      primary? true
      accept [:card_id, :body]
      change {Kanban.Changes.RelateActor, attribute: :author_id}
    end

    read :read do
      primary? true
      pagination keyset?: true, offset?: true, countable: true, required?: false
    end

    update :update_body do
      accept [:body]
    end
  end

  policies do
    policy always() do
      authorize_if always()
    end
  end
end
