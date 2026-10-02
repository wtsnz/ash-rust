defmodule Cybercab.Schema do
  use Absinthe.Schema
  use AshGraphql, domains: [Cybercab.Fleet, Cybercab.Riders, Cybercab.Rides, Cybercab.Telemetry]

  query do
  end

  mutation do
  end

  subscription do
  end
end
