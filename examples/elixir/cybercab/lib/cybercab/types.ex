defmodule Cybercab.Types.RiderTier do
  @moduledoc "How a rider rides with us."
  use Ash.Type.Enum, values: [:standard, :plus, :founder]

  def graphql_type(_), do: :rider_tier
end

defmodule Cybercab.Types.AlertKind do
  @moduledoc "What an alert is about."
  use Ash.Type.Enum,
    values: [
      :low_battery,
      :hard_braking,
      :obstruction,
      :rider_assist,
      :sensor_degraded,
      :door_ajar
    ]

  def graphql_type(_), do: :alert_kind
end

defmodule Cybercab.Types.Severity do
  @moduledoc "How urgently an alert needs a person."
  use Ash.Type.Enum, values: [:info, :warning, :critical]

  def graphql_type(_), do: :severity
end
