//! Live data for generated clients: GraphQL subscriptions over WebSocket, live queries
//! that keep a list in sync with the server, and React hooks over both.
//!
//! The runtime speaks the `graphql-transport-ws` protocol that `ash-graphql` serves at
//! `/graphql/ws`, and subscribes to the `<resource>Created`, `<resource>Updated` and
//! `<resource>Destroyed` subscriptions it generates for every resource.

use ash_core::ResourceDef;

use crate::types::to_camel_case;

/// The subscription runtime: the WebSocket client and the live-query engine.
pub fn generate_subscription_runtime(default_endpoint: &str) -> String {
    format!(
        r#"// Live data: GraphQL subscriptions over WebSocket (`graphql-transport-ws`)
export type AshConnectionStatus = "idle" | "connecting" | "connected" | "reconnecting" | "closed";

export interface AshSubscriptionSink<T> {{
  next(data: T): void;
  error?(error: AshClientError): void;
  /**
   * Events were missed: the subscriber fell behind the server, or the server ended the
   * subscription. `count` is how many, when the server says. The subscription carries on;
   * anything built from its events should be re-read. Without this, `error` hears it.
   */
  missed?(count?: number): void;
}}

interface AshActiveSubscription {{
  query: string;
  variables: Record<string, unknown>;
  sink: AshSubscriptionSink<unknown>;
  /** Times in a row the server ended it before it heard anything. */
  restarts: number;
  /** How many events the server said this subscription missed, before it ended it. */
  missed?: number;
  restartTimer?: ReturnType<typeof setTimeout>;
}}

type AshWireMessage = {{
  id?: string;
  type: string;
  payload?: unknown;
}};

/**
 * One WebSocket shared by every subscription of a client. It connects on the first
 * subscription, resubscribes everything after a dropped connection, and closes once
 * nothing is subscribed. A subscription the server ends, as it does one that falls too
 * far behind, is resubscribed and told it missed events.
 */
export class AshSubscriptionClient {{
  private readonly defaultEndpoint = "{default_endpoint}";
  private socket?: WebSocket;
  private acknowledged = false;
  private nextId = 1;
  private retries = 0;
  private retryTimer?: ReturnType<typeof setTimeout>;
  private idleTimer?: ReturnType<typeof setTimeout>;
  private status: AshConnectionStatus = "idle";
  private readonly active = new Map<string, AshActiveSubscription>();
  private readonly statusListeners = new Set<(status: AshConnectionStatus) => void>();
  private readonly connectedListeners = new Set<() => void>();

  constructor(private readonly config: AshClientConfig) {{}}

  public get connectionStatus(): AshConnectionStatus {{
    return this.status;
  }}

  /** Calls `listener` with the connection status now and whenever it changes. */
  public onStatus(listener: (status: AshConnectionStatus) => void): () => void {{
    this.statusListeners.add(listener);
    listener(this.status);
    return () => {{
      this.statusListeners.delete(listener);
    }};
  }}

  /**
   * Calls `listener` each time the server acknowledges the connection and its
   * subscriptions are sent: first, and after a dropped connection is restored. Changes
   * made before then were not heard.
   */
  public onConnected(listener: () => void): () => void {{
    this.connectedListeners.add(listener);
    return () => {{
      this.connectedListeners.delete(listener);
    }};
  }}

  public subscribe<T>(
    query: string,
    variables: Record<string, unknown>,
    sink: AshSubscriptionSink<T>,
  ): () => void {{
    const id = String(this.nextId++);
    this.active.set(id, {{ query, variables, sink: sink as AshSubscriptionSink<unknown>, restarts: 0 }});
    if (this.idleTimer) {{
      clearTimeout(this.idleTimer);
      this.idleTimer = undefined;
    }}
    if (this.acknowledged) {{
      this.send({{ id, type: "subscribe", payload: {{ query, variables }} }});
    }} else {{
      this.connect();
    }}
    return () => {{
      const subscription = this.active.get(id);
      if (!subscription) return;
      this.active.delete(id);
      if (subscription.restartTimer) clearTimeout(subscription.restartTimer);
      if (this.acknowledged) this.send({{ id, type: "complete" }});
      if (this.active.size === 0) {{
        this.idleTimer = setTimeout(() => this.close(), 1000);
      }}
    }};
  }}

  /** Closes the connection and forgets every subscription. */
  public close(): void {{
    if (this.retryTimer) clearTimeout(this.retryTimer);
    if (this.idleTimer) clearTimeout(this.idleTimer);
    this.retryTimer = undefined;
    this.idleTimer = undefined;
    const socket = this.socket;
    this.socket = undefined;
    this.acknowledged = false;
    this.retries = 0;
    socket?.close();
    this.setStatus("closed");
  }}

  private url(): string {{
    if (this.config.subscriptionUrl) return this.config.subscriptionUrl;
    const base = this.config.baseUrl.replace(/\/$/, "").replace(/^http/, "ws");
    const endpoint = this.config.subscriptionEndpoint ?? this.defaultEndpoint;
    return `${{base}}${{endpoint.startsWith("/") ? "" : "/"}}${{endpoint}}`;
  }}

  private connect(): void {{
    if (this.socket) return;
    const WebSocketImpl =
      this.config.webSocket ?? (typeof WebSocket !== "undefined" ? WebSocket : undefined);
    if (!WebSocketImpl) {{
      throw new Error("No WebSocket implementation available. Please pass `webSocket` to AshClientConfig.");
    }}
    this.setStatus(this.retries > 0 ? "reconnecting" : "connecting");
    const socket = new WebSocketImpl(this.url(), "graphql-transport-ws");
    this.socket = socket;
    socket.onopen = async () => {{
      const params = this.config.connectionParams;
      const payload = typeof params === "function" ? await params() : params;
      if (this.socket === socket) socket.send(JSON.stringify({{ type: "connection_init", payload }}));
    }};
    socket.onmessage = (event: MessageEvent) => {{
      if (this.socket === socket) this.receive(JSON.parse(String(event.data)) as AshWireMessage);
    }};
    socket.onclose = () => this.dropped(socket);
    socket.onerror = () => socket.close();
  }}

  private receive(message: AshWireMessage): void {{
    switch (message.type) {{
      case "connection_ack": {{
        this.acknowledged = true;
        this.retries = 0;
        this.setStatus("connected");
        for (const [id, subscription] of this.active) {{
          // Resubscribed here, and re-read through `onConnected`.
          if (subscription.restartTimer) clearTimeout(subscription.restartTimer);
          subscription.restartTimer = undefined;
          subscription.missed = undefined;
          this.send({{
            id,
            type: "subscribe",
            payload: {{ query: subscription.query, variables: subscription.variables }},
          }});
        }}
        for (const listener of this.connectedListeners) listener();
        break;
      }}
      case "ping":
        this.send({{ type: "pong" }});
        break;
      case "next": {{
        const subscription = message.id ? this.active.get(message.id) : undefined;
        const payload = message.payload as {{
          data?: unknown;
          errors?: Array<{{ message: string; extensions?: {{ code?: string; missed?: number }} }}>;
        }};
        if (!subscription) break;
        const missed = payload.errors?.find((error) => error.extensions?.code === "MISSED_EVENTS");
        if (missed) {{
          // The server ends the subscription next; it's resubscribed then.
          subscription.missed = missed.extensions?.missed;
        }} else if (payload.errors && payload.errors.length > 0) {{
          subscription.sink.error?.(new AshClientError(payload.errors[0].message, payload.errors));
        }} else if (payload.data !== undefined) {{
          subscription.restarts = 0;
          subscription.sink.next(payload.data);
        }}
        break;
      }}
      case "error": {{
        const subscription = message.id ? this.active.get(message.id) : undefined;
        const errors = (message.payload as Array<{{ message: string }}>) ?? [];
        if (message.id) this.active.delete(message.id);
        subscription?.sink.error?.(
          new AshClientError(errors[0]?.message ?? "Subscription failed", errors),
        );
        break;
      }}
      case "complete": {{
        // Still wanted, so the server ended it: resubscribe, then say what was missed.
        const id = message.id;
        const subscription = id ? this.active.get(id) : undefined;
        if (id && subscription) this.restart(id, subscription);
        break;
      }}
    }}
  }}

  private restart(id: string, subscription: AshActiveSubscription): void {{
    const missed = subscription.missed;
    subscription.missed = undefined;
    // Straight away after falling behind; with a growing pause if the server keeps
    // ending it before it hears anything.
    const delay = subscription.restarts === 0 ? 0 : Math.min(30_000, 250 * 2 ** subscription.restarts);
    subscription.restarts += 1;
    subscription.restartTimer = setTimeout(() => {{
      subscription.restartTimer = undefined;
      if (this.active.get(id) !== subscription || !this.acknowledged) return;
      this.send({{
        id,
        type: "subscribe",
        payload: {{ query: subscription.query, variables: subscription.variables }},
      }});
      if (subscription.sink.missed) {{
        subscription.sink.missed(missed);
      }} else {{
        subscription.sink.error?.(
          new AshClientError(
            missed === undefined ? "Subscription restarted" : `Missed ${{missed}} events`,
          ),
        );
      }}
    }}, delay);
  }}

  private dropped(socket: WebSocket): void {{
    if (this.socket !== socket) return;
    this.socket = undefined;
    this.acknowledged = false;
    if (this.active.size === 0) {{
      this.setStatus("closed");
      return;
    }}
    this.retries += 1;
    const delay = Math.min(30_000, 500 * 2 ** (this.retries - 1)) * (0.75 + Math.random() / 2);
    this.setStatus("reconnecting");
    this.retryTimer = setTimeout(() => {{
      this.retryTimer = undefined;
      this.connect();
    }}, delay);
  }}

  private send(message: AshWireMessage): void {{
    this.socket?.send(JSON.stringify(message));
  }}

  private setStatus(status: AshConnectionStatus): void {{
    if (this.status === status) return;
    this.status = status;
    for (const listener of this.statusListeners) listener(status);
  }}
}}

export interface AshSubscribeOptions {{
  onError?: (error: AshClientError) => void;
  /**
   * Called when events were missed, because this subscriber fell behind the server. It
   * carries on; re-read anything built from its events. Without it, `onError` hears it.
   */
  onMissed?: (count?: number) => void;
}}

export interface AshLiveOptions {{
  /** Called when a fetch or subscription fails; the list keeps its last value. */
  onError?: (error: unknown) => void;
  /** How long to gather changes before re-reading a filtered, sorted or paged list. */
  syncDelayMs?: number;
}}

export interface AshLiveQuery {{
  /** Re-reads the list now. */
  refresh(): Promise<void>;
  /** Stops listening. */
  stop(): void;
}}

/** What a live query needs from a resource client. */
/**
 * How the client can compare a field's values exactly as the server does, for the fields
 * it can. Text sorts by the database's collation and enums cross the wire in another
 * form, so those, like decimals and case-insensitive text, are left to the server.
 */
export type AshFieldKind = "uuid" | "text" | "number" | "boolean" | "datetime";

/** True, false, SQL's NULL (which a filter treats as false), or `undefined`: can't tell. */
type AshVerdict = boolean | null | undefined;

function ashAll(verdicts: AshVerdict[]): AshVerdict {{
  if (verdicts.some((v) => v === false)) return false;
  if (verdicts.some((v) => v === undefined)) return undefined;
  return verdicts.some((v) => v === null) ? null : true;
}}

function ashAny(verdicts: AshVerdict[]): AshVerdict {{
  if (verdicts.some((v) => v === true)) return true;
  if (verdicts.some((v) => v === undefined)) return undefined;
  return verdicts.some((v) => v === null) ? null : false;
}}

function ashFieldVerdict(kind: AshFieldKind | undefined, ops: Record<string, unknown>, value: unknown): AshVerdict {{
  if (!kind) return undefined;
  const missing = value === null || value === undefined;
  const norm = (v: unknown) => (kind === "uuid" && typeof v === "string" ? v.toLowerCase() : v);
  const verdicts: AshVerdict[] = [];
  for (const [op, operand] of Object.entries(ops)) {{
    if (operand === undefined) continue;
    if (op === "isNil") {{
      verdicts.push(missing === operand);
      continue;
    }}
    // Datetimes compare only by presence: a filter's value may be spelled another way.
    if (kind === "datetime") return undefined;
    // Null-safe equality: nulls are equal to each other and to nothing else.
    if (op === "isDistinctFrom" || op === "isNotDistinctFrom") {{
      const same = missing || operand === null ? missing && operand === null : norm(value) === norm(operand);
      verdicts.push(op === "isNotDistinctFrom" ? same : !same);
      continue;
    }}
    // A comparison with a null operand is the server's to judge.
    if (operand === null) return undefined;
    if (missing) {{
      verdicts.push(null);
      continue;
    }}
    const v = norm(value);
    switch (op) {{
      case "eq":
        verdicts.push(v === norm(operand));
        break;
      case "notEq":
        verdicts.push(v !== norm(operand));
        break;
      case "in": {{
        if (!Array.isArray(operand)) return undefined;
        const values = operand.map(norm);
        verdicts.push(values.includes(v) ? true : values.includes(null) ? null : false);
        break;
      }}
      case "lessThan":
      case "greaterThan":
      case "lessThanOrEqual":
      case "greaterThanOrEqual": {{
        if (kind !== "number" || typeof operand !== "number" || typeof v !== "number") return undefined;
        verdicts.push(
          op === "greaterThan"
            ? v > operand
            : op === "greaterThanOrEqual"
              ? v >= operand
              : op === "lessThan"
                ? v < operand
                : v <= operand,
        );
        break;
      }}
      default:
        return undefined;
    }}
  }}
  return ashAll(verdicts);
}}

function ashFilterVerdict(
  filter: Record<string, unknown>,
  record: Record<string, unknown>,
  kinds: Record<string, AshFieldKind>,
): AshVerdict {{
  const verdicts: AshVerdict[] = [];
  for (const [key, condition] of Object.entries(filter)) {{
    if (condition === undefined) continue;
    if (key === "and" || key === "or") {{
      const parts = (condition as Record<string, unknown>[]).map((part) => ashFilterVerdict(part, record, kinds));
      verdicts.push(key === "and" ? ashAll(parts) : ashAny(parts));
    }} else if (key === "not") {{
      // `not: [a, b]` excludes records matching all of them.
      const parts = (condition as Record<string, unknown>[]).map((part) => ashFilterVerdict(part, record, kinds));
      const inner = ashAll(parts);
      // NOT over SQL's NULL is NULL in Postgres; leave it to the server.
      verdicts.push(inner === true ? false : inner === false ? true : undefined);
    }} else {{
      verdicts.push(ashFieldVerdict(kinds[key], condition as Record<string, unknown>, record[key]));
    }}
  }}
  return ashAll(verdicts);
}}

/**
 * Whether `record` matches `filter` as the server would decide it, or `undefined` when
 * the client can't be sure: a relationship, an operator or a field it doesn't evaluate.
 */
export function ashMatches(
  filter: object | undefined,
  record: object,
  kinds: Record<string, AshFieldKind>,
): boolean | undefined {{
  if (!filter) return true;
  const verdict = ashFilterVerdict(filter as Record<string, unknown>, record as Record<string, unknown>, kinds);
  return verdict === undefined ? undefined : verdict === true;
}}

/**
 * The order `sort` puts two records in, as the server would, or `undefined` when the
 * client can't be sure: a field it doesn't order, or a missing value.
 */
export function ashCompare(
  sort: Array<{{ field: string; order?: "asc" | "desc" }}>,
  kinds: Record<string, AshFieldKind>,
): (a: object, b: object) => number | undefined {{
  return (a, b) => {{
    for (const {{ field, order }} of sort) {{
      const kind = kinds[field];
      if (!kind || kind === "text") return undefined;
      const x = (a as Record<string, unknown>)[field];
      const y = (b as Record<string, unknown>)[field];
      if (x === null || x === undefined || y === null || y === undefined) return undefined;
      const left = kind === "uuid" ? String(x).toLowerCase() : (x as number | string | boolean);
      const right = kind === "uuid" ? String(y).toLowerCase() : (y as number | string | boolean);
      if (left === right) continue;
      const ascending = left < right ? -1 : 1;
      return order === "desc" ? -ascending : ascending;
    }}
    return 0;
  }};
}}

export interface AshLiveSpec<T> {{
  key: (record: T) => string;
  fetch: () => Promise<T[]>;
  /** Whether a record belongs in the list: `undefined` when the client can't tell. Absent for an unfiltered list. */
  matches?: (record: T) => boolean | undefined;
  /** The list's order: `undefined` when the client can't tell. Absent for an unsorted list, whose new records go last. */
  compare?: (a: T, b: T) => number | undefined;
  /** A page (a limit): a record leaving it is replaced by one only the server knows. */
  paged: boolean;
  subscriptions: AshSubscriptionClient;
  onCreated: (handler: (record: T) => void, options: AshSubscribeOptions) => () => void;
  onUpdated: (handler: (record: T) => void, options: AshSubscribeOptions) => () => void;
  onDestroyed: (handler: (id: string) => void, options: AshSubscribeOptions) => () => void;
}}

/**
 * Keeps a list in sync with the server. It subscribes, then reads the list, and reads it
 * again whenever the connection is acknowledged, since changes made before the
 * subscriptions were live (or while a dropped connection was down) were not heard.
 * Changes apply as they arrive. A change the client can place exactly, because it can
 * evaluate the list's filter and sort as the server would, goes straight where the server
 * would put it: in, out, or to its place in the order. Any other change patches the records
 * the list holds and the list re-reads itself shortly after, as does a page (a limit),
 * since what enters it when a record leaves only the server knows.
 * If it misses events, because it fell behind the server, it re-reads straight away.
 * However fast changes arrive, the listener hears the list at most once an animation
 * frame.
 */
export function ashLiveQuery<T>(
  spec: AshLiveSpec<T>,
  listener: (items: T[]) => void,
  options?: AshLiveOptions,
): AshLiveQuery {{
  let items: T[] = [];
  let stopped = false;
  let fetching = false;
  let stale = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  // Changes arrive far faster than anyone can look at them, at scale: the listener hears
  // the list at most once an animation frame (or every 16 ms without one), with every
  // change since applied.
  /** Cancels the scheduled notification, while one is. */
  let pending: (() => void) | undefined;
  const flush = () => {{
    pending = undefined;
    if (!stopped) listener(items.slice());
  }};
  const emit = () => {{
    if (pending || stopped) return;
    if (typeof requestAnimationFrame === "function") {{
      const frame = requestAnimationFrame(flush);
      pending = () => cancelAnimationFrame(frame);
    }} else {{
      const timer = setTimeout(flush, 16);
      pending = () => clearTimeout(timer);
    }}
  }};
  // Where each record is in `items`, so a change finds its record without a search.
  let index = new Map<string, number>();
  const reindex = () => {{
    index = new Map(items.map((item, at) => [spec.key(item), at]));
  }};
  const refresh = async (): Promise<void> => {{
    if (stopped) return;
    if (fetching) {{
      stale = true;
      return;
    }}
    fetching = true;
    try {{
      items = await spec.fetch();
      reindex();
      emit();
    }} catch (error) {{
      options?.onError?.(error);
    }} finally {{
      fetching = false;
      if (stale) {{
        stale = false;
        void refresh();
      }}
    }}
  }};
  const resync = () => {{
    if (fetching) stale = true;
    if (timer || stopped) return;
    timer = setTimeout(() => {{
      timer = undefined;
      void refresh();
    }}, options?.syncDelayMs ?? 250);
  }};
  const upsert = (record: T, append: boolean) => {{
    const key = spec.key(record);
    const at = index.get(key);
    if (at !== undefined) {{
      items[at] = record;
    }} else if (append) {{
      index.set(key, items.length);
      items.push(record);
    }} else {{
      return;
    }}
    emit();
  }};

  /** Puts a created or updated record where the server would; false if it can't tell. */
  const place = (record: T): boolean => {{
    if (spec.paged) return false;
    const verdict = spec.matches ? spec.matches(record) : true;
    if (verdict === undefined) return false;
    const key = spec.key(record);
    const at = index.get(key);
    if (!verdict) {{
      if (at !== undefined) {{
        items.splice(at, 1);
        reindex();
        emit();
      }}
      return true;
    }}
    const compare = spec.compare;
    if (at !== undefined && (!compare || compare(items[at], record) === 0)) {{
      items[at] = record;
      emit();
      return true;
    }}
    if (!compare) {{
      index.set(key, items.length);
      items.push(record);
      emit();
      return true;
    }}
    const rest = at === undefined ? items : items.filter((_, i) => i !== at);
    let low = 0;
    let high = rest.length;
    while (low < high) {{
      const middle = (low + high) >> 1;
      const order = compare(rest[middle], record);
      if (order === undefined) return false;
      if (order <= 0) low = middle + 1;
      else high = middle;
    }}
    rest.splice(low, 0, record);
    items = rest;
    reindex();
    emit();
    return true;
  }};
  // A list read while changes arrived may be older than they are.
  const placed = () => {{
    if (fetching) stale = true;
  }};

  const subscribeOptions: AshSubscribeOptions = {{
    onError: (error) => options?.onError?.(error),
    // Changes it didn't hear about: read the list again.
    onMissed: () => void refresh(),
  }};
  const unsubscribes = [
    spec.onCreated((record) => {{
      if (place(record)) placed();
      else resync();
    }}, subscribeOptions),
    spec.onUpdated((record) => {{
      if (place(record)) {{
        placed();
      }} else {{
        upsert(record, false);
        resync();
      }}
    }}, subscribeOptions),
    spec.onDestroyed((id) => {{
      if (index.has(id)) {{
        items = items.filter((item) => spec.key(item) !== id);
        reindex();
        emit();
      }}
      if (spec.paged) resync();
      else placed();
    }}, subscribeOptions),
    spec.subscriptions.onConnected(() => void refresh()),
  ];
  void refresh();

  return {{
    refresh,
    stop() {{
      stopped = true;
      if (timer) clearTimeout(timer);
      pending?.();
      for (const unsubscribe of unsubscribes) unsubscribe();
    }},
  }};
}}
"#
    )
}

/// Subscription names `ash-graphql` gives a resource (e.g. `cabCreated`).
fn subscription_names(res: &ResourceDef) -> (String, String, String) {
    let lower = to_camel_case(res.name);
    (
        format!("{lower}Created"),
        format!("{lower}Updated"),
        format!("{lower}Destroyed"),
    )
}

fn primary_key(res: &ResourceDef) -> &'static str {
    res.attributes
        .iter()
        .find(|a| a.primary_key)
        .map(|a| a.name)
        .unwrap_or("id")
}

/// `onCreated`, `onUpdated` and `onDestroyed` for a resource client, over the
/// subscriptions ash-graphql serves as AshGraphql does: each takes a `filter`, and its
/// result holds the record (or, for a destroy, its id) under `created`, `updated` or
/// `destroyed`.
pub fn generate_resource_subscription_methods(res: &ResourceDef) -> String {
    let name = res.name;
    let pk = to_camel_case(primary_key(res));
    let (created, updated, destroyed) = subscription_names(res);
    format!(
        r#"
  /** Calls `handler` with each record created from now on, optionally only those matching `filter`. */
  public onCreated(
    handler: (record: {name}) => void,
    options?: AshSubscribeOptions & {{ filter?: {name}FilterInput; include?: {name}Include }},
  ): () => void {{
    const fields = build{name}SelectionSet(options?.include);
    const query = `subscription {name}Created($filter: {name}FilterInput) {{
      {created}(filter: $filter) {{
        created {{
          ${{fields}}
        }}
      }}
    }}`;
    return this.subscriptions.subscribe<{{ {created}: {{ created: {name} | null }} }}>(
      query,
      {{ filter: options?.filter }},
      {{
        next: (data) => {{
          const record = data.{created}?.created;
          if (record) handler(record);
        }},
        error: options?.onError,
        missed: options?.onMissed,
      }},
    );
  }}

  /**
   * Calls `handler` with each record updated from now on: only record `id`, or only those
   * matching `filter` once updated.
   */
  public onUpdated(
    handler: (record: {name}) => void,
    options?: AshSubscribeOptions & {{ id?: string; filter?: {name}FilterInput; include?: {name}Include }},
  ): () => void {{
    const fields = build{name}SelectionSet(options?.include);
    const query = `subscription {name}Updated($filter: {name}FilterInput) {{
      {updated}(filter: $filter) {{
        updated {{
          ${{fields}}
        }}
      }}
    }}`;
    const filter = options?.id !== undefined ? {{ ...options?.filter, {pk}: {{ eq: options.id }} }} : options?.filter;
    return this.subscriptions.subscribe<{{ {updated}: {{ updated: {name} | null }} }}>(
      query,
      {{ filter }},
      {{
        next: (data) => {{
          const record = data.{updated}?.updated;
          if (record) handler(record);
        }},
        error: options?.onError,
        missed: options?.onMissed,
      }},
    );
  }}

  /** Calls `handler` with the id of each record destroyed from now on, or only record `id`. */
  public onDestroyed(
    handler: (id: string) => void,
    options?: AshSubscribeOptions & {{ id?: string }},
  ): () => void {{
    const query = `subscription {name}Destroyed($filter: {name}FilterInput) {{
      {destroyed}(filter: $filter) {{
        destroyed
      }}
    }}`;
    const filter = options?.id !== undefined ? {{ {pk}: {{ eq: options.id }} }} : undefined;
    return this.subscriptions.subscribe<{{ {destroyed}: {{ destroyed: string | null }} }}>(
      query,
      {{ filter }},
      {{
        next: (data) => {{
          const id = data.{destroyed}?.destroyed;
          if (id) handler(id);
        }},
        error: options?.onError,
        missed: options?.onMissed,
      }},
    );
  }}
"#
    )
}

/// The fields of `res` a client can filter and sort on exactly as the server does, by kind.
fn field_kinds(res: &ResourceDef) -> String {
    use ash_core::AttrType;
    let fields: Vec<String> = res
        .attributes
        .iter()
        .filter_map(|attr| {
            let kind = match attr.ty {
                AttrType::Uuid => "uuid",
                AttrType::String => "text",
                AttrType::Integer | AttrType::Float => "number",
                AttrType::Boolean => "boolean",
                AttrType::UtcDatetime { .. } => "datetime",
                _ => return None,
            };
            Some(format!("{}: \"{kind}\"", to_camel_case(attr.name)))
        })
        .collect();
    format!("{{ {} }}", fields.join(", "))
}

/// `live()` for a resource's query builder.
pub fn generate_query_builder_live(res: &ResourceDef) -> String {
    let name = res.name;
    let pk = to_camel_case(primary_key(res));
    let kinds = field_kinds(res);
    format!(
        r#"
  /**
   * Keeps this query's results in sync with the server, calling `listener` with the
   * current list after every change. See `ashLiveQuery`.
   */
  public live(listener: (items: {name}[]) => void, options?: AshLiveOptions): AshLiveQuery {{
    const client = new {name}Client(this.transport, this.subscriptions);
    const include = this._include;
    const KINDS: Record<string, AshFieldKind> = {kinds};
    return ashLiveQuery<{name}>(
      {{
        key: (record) => String(record.{pk}),
        fetch: () => this.all(),
        matches: this._filter ? (record) => ashMatches(this._filter, record, KINDS) : undefined,
        compare: this._sort.length > 0 ? ashCompare(this._sort, KINDS) : undefined,
        paged: this._limit !== undefined,
        subscriptions: this.subscriptions,
        onCreated: (handler, opts) => client.onCreated(handler, {{ ...opts, filter: this._filter, include }}),
        onUpdated: (handler, opts) => client.onUpdated(handler, {{ ...opts, include }}),
        onDestroyed: (handler, opts) => client.onDestroyed(handler, opts),
      }},
      listener,
      options,
    );
  }}
"#
    )
}

/// `use<Resource>Live`, a React hook over a live query.
pub fn generate_resource_live_hook(res: &ResourceDef, client_name: &str) -> String {
    let name = res.name;
    let prop = to_camel_case(name);
    format!(
        r#"export interface {name}LiveParams {{
  filter?: {name}FilterInput;
  sort?: {name}SortInput[];
  limit?: number;
  include?: {name}Include;
}}

/** The records matching `params`, kept in sync with the server while mounted. */
export function use{name}Live(
  client: {client_name},
  params?: {name}LiveParams,
  options?: AshLiveOptions & {{ enabled?: boolean }},
): {{ data: {name}[]; loading: boolean; error?: unknown }} {{
  const [state, setState] = useState<{{ data: {name}[]; loading: boolean; error?: unknown }}>({{
    data: [],
    loading: true,
  }});
  const key = JSON.stringify(params ?? {{}});
  const enabled = options?.enabled ?? true;
  const syncDelayMs = options?.syncDelayMs;
  useEffect(() => {{
    if (!enabled) return;
    const builder = client.{prop}.query();
    if (params?.filter) builder.filter(params.filter);
    for (const sort of params?.sort ?? []) builder.sort(sort.field, sort.order);
    if (params?.limit !== undefined) builder.limit(params.limit);
    if (params?.include) builder.include(params.include);
    const live = builder.live((data) => setState({{ data, loading: false }}), {{
      syncDelayMs,
      onError: (error) => setState((current) => ({{ ...current, loading: false, error }})),
    }});
    return () => live.stop();
    // `key` stands for `params`, compared by value.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }}, [client, key, enabled, syncDelayMs]);
  return state;
}}

"#
    )
}

/// `useAshConnectionStatus`, the live connection's status as React state.
pub fn generate_connection_status_hook(client_name: &str) -> String {
    format!(
        r#"/// The status of the client's live connection, for a "live" indicator.
export function useAshConnectionStatus(client: {client_name}): AshConnectionStatus {{
  const [status, setStatus] = useState<AshConnectionStatus>(client.subscriptions.connectionStatus);
  useEffect(() => client.subscriptions.onStatus(setStatus), [client]);
  return status;
}}

"#
    )
}
