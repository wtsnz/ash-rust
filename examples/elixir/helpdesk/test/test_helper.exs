Ecto.Migrator.run(Helpdesk.Repo, :up, all: true, log: false)

ExUnit.start()
