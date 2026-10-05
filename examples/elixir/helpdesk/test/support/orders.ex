defmodule Helpdesk.Orders do
  @moduledoc "Orders and their line items: sum and filtered aggregates, which the desk's own resources don't use."
  use Ash.Domain, validate_config_inclusion?: false

  resources do
    resource Helpdesk.Orders.Order
    resource Helpdesk.Orders.LineItem
  end
end

defmodule Helpdesk.Orders.Order do
  @moduledoc false
  use Ash.Resource,
    domain: Helpdesk.Orders,
    data_layer: Ash.DataLayer.Ets,
    authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :customer_name, :string, allow_nil?: false, public?: true
  end

  relationships do
    has_many :line_items, Helpdesk.Orders.LineItem,
      destination_attribute: :order_id,
      public?: true
  end

  aggregates do
    count :item_count, :line_items, public?: true
    sum :total_amount, :line_items, :amount, public?: true

    sum :paid_amount, :line_items, :amount do
      public? true
      filter expr(status == "paid")
    end

    exists :has_items, :line_items, public?: true

    first :pending_item_sku, :line_items, :sku do
      public? true
      filter expr(status == "pending")
    end
  end

  actions do
    defaults [:read]

    create :create do
      primary? true
      accept [:customer_name]
    end
  end

  policies do
    policy always() do
      authorize_if always()
    end
  end
end

defmodule Helpdesk.Orders.LineItem do
  @moduledoc false
  use Ash.Resource,
    domain: Helpdesk.Orders,
    data_layer: Ash.DataLayer.Ets,
    authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :sku, :string, allow_nil?: false, public?: true
    attribute :amount, :integer, allow_nil?: false, public?: true
    attribute :status, :string, allow_nil?: false, public?: true
  end

  relationships do
    belongs_to :order, Helpdesk.Orders.Order,
      allow_nil?: false,
      public?: true,
      attribute_writable?: true
  end

  actions do
    defaults [:read]

    create :create do
      primary? true
      accept [:order_id, :sku, :amount, :status]
    end
  end

  policies do
    policy always() do
      authorize_if always()
    end
  end
end
