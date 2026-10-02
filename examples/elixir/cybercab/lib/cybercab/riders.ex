defmodule Cybercab.Riders do
  @moduledoc "The people who hail cabs."
  use Ash.Domain, extensions: [AshGraphql.Domain]

  resources do
    resource Cybercab.Riders.Rider
  end
end
