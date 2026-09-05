import React, { useState, useEffect } from "react";
import {
  ash,
  type Ticket,
  type Representative,
  type OpenTicketInput,
  OpenTicketInputSchema,
} from "../lib/client";

interface Props {
  initialTickets?: Ticket[];
  initialReps?: Representative[];
}

export default function TicketManager({
  initialTickets = [],
  initialReps = [],
}: Props) {
  const [tickets, setTickets] = useState<Ticket[]>(initialTickets);
  const [reps, setReps] = useState<Representative[]>(initialReps);
  const [statusFilter, setStatusFilter] = useState<string>("ALL");
  const [searchQuery, setSearchQuery] = useState<string>("");
  const [loading, setLoading] = useState<boolean>(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({});
  const [successMsg, setSuccessMsg] = useState<string | null>(null);

  // New ticket form state
  const [formTitle, setFormTitle] = useState("");
  const [formDesc, setFormDesc] = useState("");
  const [formPriority, setFormPriority] = useState<number>(2);
  const [formStatus, setFormStatus] = useState<"OPEN" | "IN_PROGRESS" | "RESOLVED" | "CLOSED">("OPEN");
  const [formAuthorId, setFormAuthorId] = useState<string>("");

  // Load latest data on mount if not pre-populated
  useEffect(() => {
    if (initialTickets.length === 0 || initialReps.length === 0) {
      loadData();
    }
  }, []);

  async function loadData() {
    setLoading(true);
    setErrorMsg(null);
    try {
      const [ticketList, repList] = await Promise.all([
        ash.ticket.query().include({ author: true }).sort("priority", "asc").all(),
        ash.representative.query().all(),
      ]);
      setTickets(ticketList);
      setReps(repList);
      if (repList.length > 0 && !formAuthorId) {
        setFormAuthorId(repList[0].id);
      }
    } catch (err: any) {
      setErrorMsg(err.message || "Failed to load tickets from ash-rust");
    } finally {
      setLoading(false);
    }
  }

  // --- CRUD: CREATE ---
  async function handleCreateTicket(e: React.FormEvent) {
    e.preventDefault();
    setValidationErrors({});
    setErrorMsg(null);
    setSuccessMsg(null);

    const payload: OpenTicketInput = {
      title: formTitle.trim(),
      description: formDesc.trim() || null,
      priority: Number(formPriority),
      status: formStatus,
      author_id: formAuthorId || null,
    };

    // 1. Zod Schema Validation (Derived directly from Ash Rust backend)
    const validationResult = OpenTicketInputSchema.safeParse(payload);
    if (!validationResult.success) {
      const fieldErrors: Record<string, string> = {};
      for (const issue of validationResult.error.issues) {
        const fieldName = issue.path[0]?.toString() || "form";
        fieldErrors[fieldName] = issue.message;
      }
      setValidationErrors(fieldErrors);
      return;
    }

    setLoading(true);
    try {
      // 2. Invoke Create Action via TypeScript SDK
      const newTicket = await ash.ticket.open(payload, { author: true });
      setTickets((prev) => [newTicket, ...prev]);
      setFormTitle("");
      setFormDesc("");
      setFormPriority(2);
      setSuccessMsg(`Ticket created: "${newTicket.title}"`);
      setTimeout(() => setSuccessMsg(null), 4000);
    } catch (err: any) {
      setErrorMsg(err.message || "Failed to open ticket");
    } finally {
      setLoading(false);
    }
  }

  // --- CRUD: UPDATE (Change Status) ---
  async function handleChangeStatus(
    id: string,
    newStatus: "OPEN" | "IN_PROGRESS" | "RESOLVED" | "CLOSED"
  ) {
    setLoading(true);
    setErrorMsg(null);
    try {
      const updated = await ash.ticket.changeStatus(
        id,
        { status: newStatus },
        { author: true }
      );
      setTickets((prev) => prev.map((t) => (t.id === id ? updated : t)));
      setSuccessMsg(`Status updated to ${newStatus}`);
      setTimeout(() => setSuccessMsg(null), 3000);
    } catch (err: any) {
      setErrorMsg(err.message || "Failed to update status");
    } finally {
      setLoading(false);
    }
  }

  // --- CRUD: DELETE (Close/Destroy) ---
  async function handleCloseTicket(id: string) {
    if (!confirm("Are you sure you want to close and archive this ticket?")) {
      return;
    }
    setLoading(true);
    setErrorMsg(null);
    try {
      await ash.ticket.close(id);
      setTickets((prev) => prev.filter((t) => t.id !== id));
      setSuccessMsg("Ticket closed and removed.");
      setTimeout(() => setSuccessMsg(null), 3000);
    } catch (err: any) {
      setErrorMsg(err.message || "Failed to close ticket");
    } finally {
      setLoading(false);
    }
  }

  // Filter and search
  const filteredTickets = tickets.filter((t) => {
    const matchesStatus = statusFilter === "ALL" || t.status === statusFilter;
    const matchesSearch =
      !searchQuery ||
      t.title.toLowerCase().includes(searchQuery.toLowerCase()) ||
      (t.description && t.description.toLowerCase().includes(searchQuery.toLowerCase()));
    return matchesStatus && matchesSearch;
  });

  const getPriorityBadge = (p: number) => {
    switch (p) {
      case 1:
        return <span className="badge badge-urgent">P1 Critical</span>;
      case 2:
        return <span className="badge badge-high">P2 High</span>;
      case 3:
        return <span className="badge badge-medium">P3 Medium</span>;
      default:
        return <span className="badge badge-low">P{p} Low</span>;
    }
  };

  const getStatusBadge = (s: string) => {
    switch (s) {
      case "OPEN":
        return <span className="badge badge-open">OPEN</span>;
      case "IN_PROGRESS":
        return <span className="badge badge-progress">IN PROGRESS</span>;
      case "RESOLVED":
        return <span className="badge badge-resolved">RESOLVED</span>;
      case "CLOSED":
        return <span className="badge badge-closed">CLOSED</span>;
      default:
        return <span className="badge">{s}</span>;
    }
  };

  return (
    <div className="ticket-manager">
      {/* Toast notifications */}
      {successMsg && <div className="alert alert-success">{successMsg}</div>}
      {errorMsg && <div className="alert alert-error">{errorMsg}</div>}

      {/* Main Grid: Create Form on Left, Tickets List on Right */}
      <div className="grid-layout">
        {/* CREATE FORM */}
        <div className="card form-card">
          <div className="card-header">
            <h3>New Support Ticket</h3>
            <span className="badge-tech">Zod Validated</span>
          </div>
          <form onSubmit={handleCreateTicket} noValidate>
            <div className="form-group">
              <label htmlFor="title">
                Issue Title <span className="req">* (min 5 chars)</span>
              </label>
              <input
                id="title"
                type="text"
                value={formTitle}
                onChange={(e) => setFormTitle(e.target.value)}
                placeholder="e.g. Memory leak in background worker"
                className={validationErrors.title ? "input-error" : ""}
              />
              {validationErrors.title && (
                <p className="err-msg">{validationErrors.title}</p>
              )}
            </div>

            <div className="form-group">
              <label htmlFor="desc">Description</label>
              <textarea
                id="desc"
                value={formDesc}
                onChange={(e) => setFormDesc(e.target.value)}
                placeholder="Detailed steps or stack trace..."
                rows={3}
              />
            </div>

            <div className="form-row">
              <div className="form-group">
                <label htmlFor="priority">Priority (1-5)</label>
                <select
                  id="priority"
                  value={formPriority}
                  onChange={(e) => setFormPriority(Number(e.target.value))}
                >
                  <option value={1}>P1 - Urgent</option>
                  <option value={2}>P2 - High</option>
                  <option value={3}>P3 - Medium</option>
                  <option value={4}>P4 - Low</option>
                  <option value={5}>P5 - Trivial</option>
                </select>
                {validationErrors.priority && (
                  <p className="err-msg">{validationErrors.priority}</p>
                )}
              </div>

              <div className="form-group">
                <label htmlFor="status">Initial Status</label>
                <select
                  id="status"
                  value={formStatus}
                  onChange={(e) => setFormStatus(e.target.value as any)}
                >
                  <option value="OPEN">OPEN</option>
                  <option value="IN_PROGRESS">IN_PROGRESS</option>
                  <option value="RESOLVED">RESOLVED</option>
                </select>
              </div>
            </div>

            <div className="form-group">
              <label htmlFor="author">Assigned Representative</label>
              <select
                id="author"
                value={formAuthorId}
                onChange={(e) => setFormAuthorId(e.target.value)}
              >
                <option value="">(Unassigned)</option>
                {reps.map((rep) => (
                  <option key={rep.id} value={rep.id}>
                    {rep.name} ({rep.role})
                  </option>
                ))}
              </select>
            </div>

            <button
              type="submit"
              disabled={loading}
              className="btn btn-primary btn-block"
            >
              {loading ? "Submitting to Rust backend..." : "+ Open Ticket"}
            </button>
          </form>
        </div>

        {/* TICKETS LIST (READ, UPDATE, DELETE) */}
        <div className="card list-card">
          <div className="list-toolbar">
            <div className="filters">
              {["ALL", "OPEN", "IN_PROGRESS", "RESOLVED"].map((status) => (
                <button
                  key={status}
                  onClick={() => setStatusFilter(status)}
                  className={`tab-btn ${statusFilter === status ? "active" : ""}`}
                >
                  {status}
                </button>
              ))}
            </div>

            <div className="search-box">
              <input
                type="text"
                placeholder="Search tickets..."
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
              />
              <button onClick={loadData} title="Refresh" className="btn-refresh">
                🔄
              </button>
            </div>
          </div>

          <div className="tickets-container">
            {filteredTickets.length === 0 ? (
              <div className="empty-state">
                <p>No tickets match the selected criteria.</p>
              </div>
            ) : (
              filteredTickets.map((t) => (
                <div key={t.id} className="ticket-item">
                  <div className="ticket-top">
                    <div className="badges">
                      {getPriorityBadge(t.priority)}
                      {getStatusBadge(t.status)}
                    </div>
                    <span className="ticket-id">#{t.id.slice(0, 8)}</span>
                  </div>

                  <h4 className="ticket-title">{t.title}</h4>
                  {t.description && (
                    <p className="ticket-desc">{t.description}</p>
                  )}

                  <div className="ticket-bottom">
                    <div className="ticket-author">
                      {t.author ? (
                        <span>
                          👤 <strong>{t.author.name}</strong> ({t.author.role})
                        </span>
                      ) : (
                        <span className="unassigned">Unassigned</span>
                      )}
                    </div>

                    {/* Actions: Update status and Delete */}
                    <div className="ticket-actions">
                      {t.status !== "IN_PROGRESS" && t.status !== "RESOLVED" && (
                        <button
                          onClick={() => handleChangeStatus(t.id, "IN_PROGRESS")}
                          className="btn-action btn-progress"
                        >
                          Start Work
                        </button>
                      )}
                      {t.status !== "RESOLVED" && (
                        <button
                          onClick={() => handleChangeStatus(t.id, "RESOLVED")}
                          className="btn-action btn-resolve"
                        >
                          Resolve
                        </button>
                      )}
                      {t.status === "RESOLVED" && (
                        <button
                          onClick={() => handleChangeStatus(t.id, "OPEN")}
                          className="btn-action btn-reopen"
                        >
                          Reopen
                        </button>
                      )}
                      <button
                        onClick={() => handleCloseTicket(t.id)}
                        className="btn-action btn-danger"
                        title="Destroy Ticket"
                      >
                        🗑️ Close
                      </button>
                    </div>
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
