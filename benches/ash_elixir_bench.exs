Mix.install(
  [
    {:ash, "~> 3.0"},
    # Policies need a SAT solver.
    {:picosat_elixir, "~> 0.2"},
    {:benchee, "~> 1.0"}
  ],
  config: [ash: [default_string_length_count: :codepoints]]
)

Code.require_file("embedded_mode.exs", __DIR__)

Logger.configure(level: :warning)

# The Rust helpdesk's resources, policies included: a customer reads and closes the tickets
# they opened, a representative reads the unassigned queue and what's assigned to them. Every
# action runs through them, as the Rust desk's do.
defmodule Helpdesk.Support.Ticket do
  use Ash.Resource,
    domain: Helpdesk.Support,
    data_layer: Ash.DataLayer.Ets,
    authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :subject, :string, allow_nil?: false, public?: true
    attribute :status, :string, default: "open", public?: true
    attribute :opener_id, :uuid, public?: true
  end

  relationships do
    belongs_to :representative, Helpdesk.Support.Representative, public?: true, attribute_writable?: true
  end

  actions do
    defaults [:read]

    create :open do
      accept [:subject]
      validate present(:subject)
      validate string_length(:subject, min: 2)
      change set_attribute(:status, "open")

      change fn changeset, %{actor: actor} ->
        Ash.Changeset.force_change_attribute(changeset, :opener_id, actor.id)
      end
    end

    update :assign do
      accept [:representative_id]
    end
  end

  policies do
    policy action(:open) do
      authorize_if actor_present()
    end

    policy action_type(:read) do
      authorize_if expr(opener_id == ^actor(:id))
      authorize_if expr(representative_id == ^actor(:id))
      authorize_if expr(is_nil(representative_id) and ^actor(:role) == "representative")
    end

    policy action(:assign) do
      authorize_if actor_attribute_equals(:role, "representative")
    end
  end
end

defmodule Helpdesk.Support.Representative do
  use Ash.Resource,
    domain: Helpdesk.Support,
    data_layer: Ash.DataLayer.Ets,
    authorizers: [Ash.Policy.Authorizer]

  attributes do
    uuid_primary_key :id
    attribute :name, :string, allow_nil?: false, public?: true
  end

  relationships do
    has_many :tickets, Helpdesk.Support.Ticket, destination_attribute: :representative_id, public?: true
  end

  aggregates do
    count :ticket_count, :tickets, public?: true

    count :open_ticket_count, :tickets do
      public? true
      filter expr(status == "open")
    end

    exists :has_tickets, :tickets, public?: true
  end

  actions do
    defaults [:read]

    create :create do
      accept [:name]
      validate present(:name)
      validate string_length(:name, min: 2)
    end
  end

  policies do
    policy always() do
      authorize_if always()
    end
  end
end

defmodule Helpdesk.Support do
  use Ash.Domain, validate_config_inclusion?: false

  resources do
    resource Helpdesk.Support.Ticket do
      define :open_ticket, action: :open, args: [:subject]
      define :assign_ticket, action: :assign, args: [:representative_id]
      define :list_tickets, action: :read
    end

    resource Helpdesk.Support.Representative do
      define :create_representative, action: :create, args: [:name]
      define :list_representatives, action: :read
    end
  end
end

defmodule BenchRunner do
  require Ash.Query

  # Each workload starts from empty tables, as the Rust runner makes each a store of its own.
  # Emptied in place: `Ash.DataLayer.Ets.stop/1` leaves a table's manager believing in a
  # table it has deleted.
  def reset do
    for resource <- [Helpdesk.Support.Ticket, Helpdesk.Support.Representative] do
      table = Ash.DataLayer.Ets.Info.table(resource)
      if :ets.whereis(table) != :undefined, do: :ets.delete_all_objects(table)
    end
  end

  def customer, do: %{id: Ash.UUID.generate(), role: "customer"}

  def run do
    opts = [warmup: 2, time: 5, memory_time: 2, print: [fast_warning: false]]

    # Prime every measured path so :embedded mode can refuse autoload.
    reset()
    customer = customer()
    _ = Helpdesk.Support.open_ticket!("Printer is broken", actor: customer)
    _ = Helpdesk.Support.create_representative!("Alice Smith")
    _ = Helpdesk.Support.list_tickets!(actor: customer)
    query = Ash.Query.filter(Helpdesk.Support.Ticket, status == "open")
    _ = Ash.read!(query, actor: customer)
    _ = Ash.read!(Ash.Query.load(Helpdesk.Support.Representative, [:ticket_count, :open_ticket_count, :has_tickets]), actor: customer)

    AshBench.EmbeddedMode.enter!()

    IO.puts("\n=== [1/4] Ash Elixir: Ticket.open ===")
    reset()

    Benchee.run(
      %{
        "Ticket.open (policy + validate + changeset + ETS)" => fn ->
          Helpdesk.Support.open_ticket!("Printer is broken", actor: customer)
        end
      },
      opts
    )

    IO.puts("\n=== [2/4] Ash Elixir: Representative.create ===")
    reset()

    Benchee.run(
      %{
        "Representative.create (validate + ETS)" => fn ->
          Helpdesk.Support.create_representative!("Alice Smith")
        end
      },
      opts
    )

    IO.puts("\n=== [3/4] Ash Elixir: Ticket.read (100 records) ===")
    reset()
    reader = customer()

    for i <- 1..100 do
      Helpdesk.Support.open_ticket!("Ticket number #{i}", actor: reader)
    end

    Benchee.run(
      %{
        "Ticket.read (policy + filter status == open, 100 records)" => fn ->
          Ash.read!(query, actor: reader)
        end
      },
      opts
    )

    IO.puts("\n=== [4/4] Ash Elixir: load aggregates (20 assigned tickets) ===")
    reset()
    bob = Helpdesk.Support.create_representative!("Bob Jones")
    as_bob = %{id: bob.id, role: "representative"}
    reader = customer()

    for i <- 1..20 do
      ticket = Helpdesk.Support.open_ticket!("Ticket #{i}", actor: reader)
      Helpdesk.Support.assign_ticket!(ticket, bob.id, actor: as_bob)
    end

    aggregates =
      Ash.Query.load(Helpdesk.Support.Representative, [:ticket_count, :open_ticket_count, :has_tickets])

    Benchee.run(
      %{
        "load_aggregates (ticket_count, open_ticket_count, has_tickets)" => fn ->
          Ash.read!(aggregates, actor: reader)
        end
      },
      opts
    )

    AshBench.EmbeddedMode.restore!()
  end
end

BenchRunner.run()
