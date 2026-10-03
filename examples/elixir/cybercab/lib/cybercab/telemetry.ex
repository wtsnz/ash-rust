defmodule Cybercab.Telemetry do
  @moduledoc "What the fleet reports, and what the control room watches."
  use Ash.Domain, extensions: [AshGraphql.Domain]

  resources do
    resource Cybercab.Telemetry.TelemetrySample
    resource Cybercab.Telemetry.FleetAlert
    resource Cybercab.Telemetry.PulseSample
  end
end
