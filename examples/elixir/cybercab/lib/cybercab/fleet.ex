defmodule Cybercab.Fleet do
  @moduledoc "The cabs, and the Supercharger hubs they return to."
  use Ash.Domain, extensions: [AshGraphql.Domain]

  resources do
    resource Cybercab.Fleet.Cab
    resource Cybercab.Fleet.Depot
  end
end
