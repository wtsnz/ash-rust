# The Rust helpdesk's benchmarks (`examples/helpdesk/examples/bench.rs` and its Criterion
# suite), run on this desk: the same workloads, with the same policies, on ETS and on SQLite.
#
#   mise exec elixir erlang -- mix run bench/core.exs
#
# Unlike `benches/ash_elixir_bench.exs`, whose resources carry no policies, every action
# here runs through the policies the Rust desk's do.
require Ash.Query

Ecto.Migrator.run(Helpdesk.Repo, :up, all: true, log: false)
Helpdesk.Repo.delete_all(Helpdesk.Sqlite.Ticket)
Helpdesk.Repo.delete_all(Helpdesk.Sqlite.Representative)

opts = [warmup: 2, time: 5, memory_time: 2, print: [fast_warning: false]]

defmodule Core do
  # What the Rust bench does by making each workload a store of its own: empty both.
  def reset do
    # Emptied in place: `Ash.DataLayer.Ets.stop/1` leaves the table's manager believing
    # in a table it has deleted, so the next use fails.
    for resource <- [Helpdesk.Memory.Ticket, Helpdesk.Memory.Representative] do
      table = Ash.DataLayer.Ets.Info.table(resource)
      if :ets.whereis(table) != :undefined, do: :ets.delete_all_objects(table)
    end

    Helpdesk.Repo.delete_all(Helpdesk.Sqlite.Ticket)
    Helpdesk.Repo.delete_all(Helpdesk.Sqlite.Representative)
  end

  def seed(desk, count) do
    customer = Helpdesk.Actor.customer(Ash.UUID.generate())
    for i <- 1..count, do: desk.open_ticket!("Ticket number #{i}", actor: customer)
    customer
  end

  def seed_assigned(desk, count) do
    rep = desk.create_representative!("Bob Jones")
    customer = Helpdesk.Actor.customer(Ash.UUID.generate())
    as_rep = Helpdesk.Actor.representative(rep.id)

    for i <- 1..count do
      ticket = desk.open_ticket!("Ticket #{i}", actor: customer)
      desk.assign_ticket!(ticket, rep.id, actor: as_rep)
    end

    customer
  end
end

for {label, desk, ticket} <- [
      {"ETS", Helpdesk.Memory, Helpdesk.Memory.Ticket},
      {"SQLite", Helpdesk.Sqlite, Helpdesk.Sqlite.Ticket}
    ] do
  Core.reset()
  customer = Helpdesk.Actor.customer(Ash.UUID.generate())
  # Prime every measured path before timing.
  desk.open_ticket!("Printer is broken", actor: customer)
  desk.create_representative!("Alice Smith")

  IO.puts("\n=== Ash Elixir (#{label}): Ticket.open ===")

  Benchee.run(
    %{"Ticket.open (policy + validate + changeset)" => fn ->
        desk.open_ticket!("Printer is broken", actor: customer)
      end},
    opts
  )

  IO.puts("\n=== Ash Elixir (#{label}): Representative.create ===")

  Benchee.run(
    %{"Representative.create (validate)" => fn -> desk.create_representative!("Alice Smith") end},
    opts
  )

  Core.reset()
  reader = Core.seed(desk, 100)
  query = Ash.Query.filter(ticket, status == :open)

  IO.puts("\n=== Ash Elixir (#{label}): Ticket.read (filter status == open, 100+ records) ===")

  Benchee.run(
    %{"Ticket.read (policy + filter)" => fn -> Ash.read!(query, actor: reader) end},
    opts
  )
end

# AshSqlite serves no resource aggregates, so this one runs on ETS alone.
Core.reset()
reader = Core.seed_assigned(Helpdesk.Memory, 20)

query =
  Ash.Query.load(Helpdesk.Memory.Representative, [:ticket_count, :open_ticket_count, :has_tickets])

IO.puts("\n=== Ash Elixir (ETS): load aggregates (20 assigned tickets) ===")

Benchee.run(
  %{"load_aggregates (ticket_count, open_ticket_count, has_tickets)" => fn ->
      Ash.read!(query, actor: reader)
    end},
  opts
)
