defmodule Kanban.Fragments.List do
  @moduledoc "What a list is: a column of cards on a board. See `Kanban.Fragments.User`."
  use Spark.Dsl.Fragment, of: Ash.Resource, authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :board_id, :uuid, allow_nil?: false, public?: true
    attribute :title, :string, allow_nil?: false, public?: true
    attribute :position, :integer, allow_nil?: false, public?: true
    attribute :archived, :boolean, allow_nil?: false, default: false, public?: true
  end

  actions do
    defaults [:destroy]

    create :create do
      primary? true
      accept [:board_id, :title, :position]
      validate numericality(:position, greater_than_or_equal_to: 0)
      change set_attribute(:archived, false)
    end

    read :read do
      primary? true
      pagination keyset?: true, offset?: true, countable: true, required?: false
    end

    update :rename do
      accept [:title]
    end

    update :move_position do
      accept [:position]
      validate numericality(:position, greater_than_or_equal_to: 0)
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
