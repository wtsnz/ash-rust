import Config

config :cybercab,
  ecto_repos: [Cybercab.Repo],
  ash_domains: [Cybercab.Fleet, Cybercab.Riders, Cybercab.Rides, Cybercab.Telemetry]

config :ash, :disable_async?, false

# Count string length in codepoints, as SQL data layers do.
config :ash, default_string_length_count: :codepoints
