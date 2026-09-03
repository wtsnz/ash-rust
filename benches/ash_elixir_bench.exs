Mix.install([
  {:ash, "~> 3.0"},
  {:benchee, "~> 1.0"}
])

Logger.configure(level: :warning)

defmodule Helpdesk.Support.Ticket do
  use Ash.Resource,
    domain: Helpdesk.Support,
    data_layer: Ash.DataLayer.Ets

  attributes do
    uuid_primary_key :id
    attribute :subject, :string, allow_nil?: false, public?: true
    attribute :status, :string, default: "open", public?: true
  end

  actions do
    defaults [:read]

    create :open do
      accept [:subject]
      validate present(:subject)
      validate string_length(:subject, min: 2)
      change set_attribute(:status, "open")
    end
  end
end

defmodule Helpdesk.Support.Representative do
  use Ash.Resource,
    domain: Helpdesk.Support,
    data_layer: Ash.DataLayer.Ets

  attributes do
    uuid_primary_key :id
    attribute :name, :string, allow_nil?: false, public?: true
  end

  actions do
    defaults [:read]

    create :create do
      accept [:name]
      validate present(:name)
      validate string_length(:name, min: 2)
    end
  end
end

defmodule Helpdesk.Support.StaticTicket do
  use Ash.Resource,
    domain: Helpdesk.Support,
    data_layer: Ash.DataLayer.Ets

  attributes do
    uuid_primary_key :id
    attribute :subject, :string, allow_nil?: false, public?: true
    attribute :status, :string, default: "open", public?: true
  end

  actions do
    defaults [:read]
    create :create do
      accept [:subject, :status]
    end
  end
end

defmodule Helpdesk.Support do
  use Ash.Domain, validate_config_inclusion?: false

  resources do
    resource Helpdesk.Support.Ticket do
      define :open_ticket, action: :open, args: [:subject]
      define :list_tickets, action: :read
    end

    resource Helpdesk.Support.Representative do
      define :create_representative, action: :create, args: [:name]
      define :list_representatives, action: :read
    end

    resource Helpdesk.Support.StaticTicket do
      define :create_static_ticket, action: :create, args: [:subject, :status]
      define :list_static_tickets, action: :read
    end
  end
end

defmodule BenchRunner do
  require Ash.Query

  def run do
    IO.puts("\n=== [1/3] Ash Elixir: Ticket.open ===")
    Benchee.run(
      %{
        "Ticket.open (validate + changeset + ETS)" => fn ->
          Helpdesk.Support.open_ticket!("Printer is broken")
        end
      },
      warmup: 2,
      time: 5,
      memory_time: 2,
      print: [fast_warning: false]
    )

    IO.puts("\n=== [2/3] Ash Elixir: Representative.create ===")
    Benchee.run(
      %{
        "Representative.create (validate + ETS)" => fn ->
          Helpdesk.Support.create_representative!("Alice Smith")
        end
      },
      warmup: 2,
      time: 5,
      memory_time: 2,
      print: [fast_warning: false]
    )

    # Pre-populate exactly 100 records in StaticTicket
    for i <- 1..100 do
      Helpdesk.Support.create_static_ticket!("Ticket number #{i}", "open")
    end

    IO.puts("\n=== [3/3] Ash Elixir: Ticket.read (100 records) ===")
    query = Ash.Query.filter(Helpdesk.Support.StaticTicket, status == "open")

    Benchee.run(
      %{
        "Ticket.read (filter status == open, 100 records)" => fn ->
          Ash.read!(query)
        end
      },
      warmup: 2,
      time: 5,
      memory_time: 2,
      print: [fast_warning: false]
    )
  end
end

BenchRunner.run()
