Ecto.Migrator.run(Kanban.Repo, :up, all: true, log: false)

ExUnit.start()
