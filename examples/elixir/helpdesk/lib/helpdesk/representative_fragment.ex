defmodule Helpdesk.RepresentativeFragment do
  @moduledoc "What a support representative is, whatever stores them: see `Helpdesk.TicketFragment`."
  use Spark.Dsl.Fragment, of: Ash.Resource, authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :name, :string, allow_nil?: false, public?: true
  end

  actions do
    defaults [:destroy]

    create :create do
      primary? true
      accept [:name]
      validate string_length(:name, min: 2)
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
