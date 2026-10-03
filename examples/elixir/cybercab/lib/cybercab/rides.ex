defmodule Cybercab.Rides do
  @moduledoc "Trips, the core of the business, and the zones demand is tracked in."
  use Ash.Domain, extensions: [AshGraphql.Domain]

  resources do
    resource Cybercab.Rides.Trip
    resource Cybercab.Rides.ServiceZone
  end
end
