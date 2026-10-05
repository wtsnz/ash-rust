defmodule Kanban.Sqlite.Workspaces do
  @moduledoc "The workspaces domain, on SQLite."
  use Ash.Domain

  resources do
    resource Kanban.Sqlite.User do
      define :register_user, action: :register, args: [:name, :email]
      define :get_user, action: :read, get_by: :id
      define :list_users, action: :read
    end

    resource Kanban.Sqlite.Workspace do
      define :create_workspace, action: :create, args: [:name, :slug]
      define :get_workspace, action: :read, get_by: :id
      define :list_workspaces, action: :read
    end

    resource Kanban.Sqlite.WorkspaceMember do
      define :add_member, action: :add, args: [:workspace_id, :user_id, :role]
      define :remove_member, action: :destroy
      define :list_members, action: :read
    end
  end
end
