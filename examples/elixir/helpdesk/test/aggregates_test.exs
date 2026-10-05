defmodule Helpdesk.AggregatesTest do
  @moduledoc """
  Aggregates over the tickets a representative holds. Ports `tests/aggregates.rs`, on ETS
  alone: AshSqlite serves no resource aggregates, which ash-rust's SQLite does.
  """
  use Helpdesk.DeskCase, layer: Helpdesk.Memory

  alias Helpdesk.Orders.{LineItem, Order}

  defp desk_with_three_representatives do
    alice = @desk.create_representative!("Alice")
    bob = @desk.create_representative!("Bob")
    carol = @desk.create_representative!("Carol")
    as_customer = customer()
    as_alice = representative(alice)
    as_bob = representative(bob)

    # Alice gets two tickets, one of them closed.
    t1 = @desk.open_ticket!("Printer broke", actor: as_customer)
    @desk.assign_ticket!(t1, alice.id, actor: as_alice)
    t2 = @desk.open_ticket!("Wifi down", actor: as_customer)
    t2 = @desk.assign_ticket!(t2, alice.id, actor: as_alice)
    @desk.close_ticket!(t2, actor: as_alice)

    # Bob gets one.
    t3 = @desk.open_ticket!("Need new monitor", actor: as_customer)
    @desk.assign_ticket!(t3, bob.id, actor: as_bob)

    {as_customer, [alice, bob, carol]}
  end

  test "aggregates are not loaded until asked" do
    {as_customer, _} = desk_with_three_representatives()

    [first | _] = @representative |> Ash.Query.sort(:name) |> Ash.read!(actor: as_customer)
    assert %Ash.NotLoaded{} = first.ticket_count
    assert %Ash.NotLoaded{} = first.has_tickets
  end

  test "aggregates load with the representatives" do
    {as_customer, _} = desk_with_three_representatives()

    [alice, bob, carol] =
      @representative
      |> Ash.Query.sort(:name)
      |> Ash.Query.load([:ticket_count, :open_ticket_count, :has_tickets, :first_ticket_subject])
      |> Ash.read!(actor: as_customer)

    assert {alice.name, alice.ticket_count, alice.open_ticket_count, alice.has_tickets} ==
             {"Alice", 2, 1, true}

    assert alice.first_ticket_subject in ["Printer broke", "Wifi down"]

    assert {bob.name, bob.ticket_count, bob.open_ticket_count, bob.has_tickets} ==
             {"Bob", 1, 1, true}

    assert bob.first_ticket_subject == "Need new monitor"

    assert {carol.name, carol.ticket_count, carol.open_ticket_count, carol.has_tickets} ==
             {"Carol", 0, 0, false}

    assert carol.first_ticket_subject == nil
  end

  test "aggregates filter and sort" do
    {as_customer, _} = desk_with_three_representatives()

    active =
      @representative
      |> Ash.Query.filter(ticket_count > 0)
      |> Ash.Query.sort(ticket_count: :desc)
      |> Ash.Query.load(:ticket_count)
      |> Ash.read!(actor: as_customer)

    assert Enum.map(active, &{&1.name, &1.ticket_count}) == [{"Alice", 2}, {"Bob", 1}]
  end

  test "aggregates count only the tickets the reader may read" do
    {_as_customer, _} = desk_with_three_representatives()

    # Another customer opened none of them, so reads none.
    counts =
      @representative
      |> Ash.Query.load(:ticket_count)
      |> Ash.read!(actor: customer())
      |> Enum.map(& &1.ticket_count)

    assert counts == [0, 0, 0]
  end

  test "sum, filtered and first aggregates over line items" do
    order = Ash.create!(Order, %{customer_name: "Will"})

    for {sku, amount, status} <- [
          {"KEYBOARD-RGB", 120, "paid"},
          {"MOUSE-WIRELESS", 80, "paid"},
          {"MONITOR-ARM", 150, "pending"}
        ] do
      Ash.create!(LineItem, %{order_id: order.id, sku: sku, amount: amount, status: status})
    end

    loaded =
      Ash.get!(Order, order.id,
        load: [:item_count, :total_amount, :paid_amount, :has_items, :pending_item_sku]
      )

    assert loaded.item_count == 3
    assert loaded.total_amount == 350
    assert loaded.paid_amount == 200
    assert loaded.has_items == true
    assert loaded.pending_item_sku == "MONITOR-ARM"
  end
end
