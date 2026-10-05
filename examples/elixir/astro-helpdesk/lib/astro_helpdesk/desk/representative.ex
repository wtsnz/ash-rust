defmodule AstroHelpdesk.Desk.Representative do
  @moduledoc "A support representative."
  use Ash.Resource,
    domain: AstroHelpdesk.Desk,
    data_layer: Ash.DataLayer.Ets,
    extensions: [AshGraphql.Resource],
    notifiers: [Ash.Notifier.PubSub]

  attributes do
    uuid_primary_key :id
    attribute :name, :string, allow_nil?: false, public?: true
    attribute :email, :string, allow_nil?: false, public?: true
    attribute :role, :string, allow_nil?: false, public?: true
  end

  actions do
    defaults [:read]

    create :create do
      primary? true
      accept [:name, :email, :role]
    end
  end

  graphql do
    type :representative

    queries do
      get :get_representative, :read
      list :list_representatives, :read, paginate_with: :keyset
    end

    mutations do
      create :create_representative, :create
    end

    subscriptions do
      pubsub AstroHelpdeskWeb.Endpoint

      subscribe :representative_created do
        action_types [:create]
      end

      subscribe :representative_updated do
        action_types [:update]
      end

      subscribe :representative_destroyed do
        action_types [:destroy]
      end
    end
  end
end
