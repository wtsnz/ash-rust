defmodule Kanban.Memory.Workspaces do
  @moduledoc "The workspaces domain, on ETS."
  use Ash.Domain

  resources do
    resource Kanban.Memory.User do
      define :register_user, action: :register, args: [:name, :email]
      define :get_user, action: :read, get_by: :id
      define :list_users, action: :read
    end

    resource Kanban.Memory.Workspace do
      define :create_workspace, action: :create, args: [:name, :slug]
      define :get_workspace, action: :read, get_by: :id
      define :list_workspaces, action: :read
    end

    resource Kanban.Memory.WorkspaceMember do
      define :add_member, action: :add, args: [:workspace_id, :user_id, :role]
      define :remove_member, action: :destroy
      define :list_members, action: :read
    end
  end
end
