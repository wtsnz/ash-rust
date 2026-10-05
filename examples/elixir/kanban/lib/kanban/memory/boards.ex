defmodule Kanban.Memory.Boards do
  @moduledoc "The boards domain, on ETS."
  use Ash.Domain

  resources do
    resource Kanban.Memory.Board do
      define :create_board,
        action: :create,
        args: [:workspace_id, :name, {:optional, :description}]

      define :get_board, action: :read, get_by: :id
      define :list_boards, action: :read
      define :archive_board, action: :archive
    end

    resource Kanban.Memory.List do
      define :create_list, action: :create, args: [:board_id, :title, :position]
      define :get_list, action: :read, get_by: :id
      define :list_lists, action: :read
      define :rename_list, action: :rename, args: [:title]
      define :move_list, action: :move_position, args: [:position]
      define :archive_list, action: :archive
    end

    resource Kanban.Memory.Card do
      define :create_card,
        action: :create,
        args: [:board_id, :list_id, :title, {:optional, :description}, :position]

      define :get_card, action: :read, get_by: :id
      define :list_cards, action: :read
      define :move_card, action: :move_to_list, args: [:list_id, :position]
      define :assign_card, action: :assign, args: [:assignee_id]
      define :archive_card, action: :archive
    end

    resource Kanban.Memory.ChecklistItem do
      define :add_checklist_item, action: :create, args: [:card_id, :title, :position]
      define :toggle_checklist_item, action: :toggle, args: [:completed]
      define :delete_checklist_item, action: :destroy
      define :list_checklist_items, action: :read
    end

    resource Kanban.Memory.Comment do
      define :add_comment, action: :create, args: [:card_id, :body]
      define :update_comment, action: :update_body, args: [:body]
      define :delete_comment, action: :destroy
      define :list_comments, action: :read
    end
  end
end
