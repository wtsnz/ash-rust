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
export interface AshLiveSpec<T> {{
  key: (record: T) => string;
  fetch: () => Promise<T[]>;
  /** Whether events alone keep the list exact: no filter, sort, limit or offset. */
  exact: boolean;
  subscriptions: AshSubscriptionClient;
  onCreated: (handler: (record: T) => void, options: AshSubscribeOptions) => () => void;
  onUpdated: (handler: (record: T) => void, options: AshSubscribeOptions) => () => void;
  onDestroyed: (handler: (id: string) => void, options: AshSubscribeOptions) => () => void;
}}

/**
 * Keeps a list in sync with the server. It subscribes, then reads the list, and reads it
 * again whenever the connection is acknowledged, since changes made before the
 * subscriptions were live (or while a dropped connection was down) were not heard.
 * Changes apply as they arrive: an exact list (no filter, sort or paging) is patched in
 * place; any other list patches the records it holds and re-reads itself shortly after,
 * so records that enter or leave it, or move within it, land where the server puts them.
 * If it misses events, because it fell behind the server, it re-reads straight away.
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

  const emit = () => {{
    if (!stopped) listener(items.slice());
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
    if (spec.exact || timer || stopped) return;
    timer = setTimeout(() => {{
      timer = undefined;
      void refresh();
    }}, options?.syncDelayMs ?? 250);
  }};
  const upsert = (record: T, append: boolean) => {{
    const key = spec.key(record);
    const at = items.findIndex((item) => spec.key(item) === key);
    if (at >= 0) {{
      items = items.map((item, index) => (index === at ? record : item));
    }} else if (append) {{
      items = [...items, record];
    }} else {{
      return;
    }}
    emit();
  }};

  const subscribeOptions: AshSubscribeOptions = {{
    onError: (error) => options?.onError?.(error),
    // Changes it didn't hear about: read the list again.
    onMissed: () => void refresh(),
  }};
  const unsubscribes = [
    spec.onCreated((record) => {{
      if (spec.exact) upsert(record, true);
      resync();
    }}, subscribeOptions),
    spec.onUpdated((record) => {{
      upsert(record, spec.exact);
      resync();
    }}, subscribeOptions),
    spec.onDestroyed((id) => {{
      const before = items.length;
      items = items.filter((item) => spec.key(item) !== id);
      if (items.length !== before) emit();
      resync();
    }}, subscribeOptions),
    spec.subscriptions.onConnected(() => void refresh()),
  ];
  void refresh();

  return {{
    refresh,
    stop() {{
      stopped = true;
      if (timer) clearTimeout(timer);
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

/// `onCreated`, `onUpdated` and `onDestroyed` for a resource client.
pub fn generate_resource_subscription_methods(res: &ResourceDef) -> String {
    let name = res.name;
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
        ${{fields}}
      }}
    }}`;
    return this.subscriptions.subscribe<{{ {created}: {name} }}>(query, {{ filter: options?.filter }}, {{
      next: (data) => handler(data.{created}),
      error: options?.onError,
      missed: options?.onMissed,
    }});
  }}

  /** Calls `handler` with each record updated from now on, or only record `id`. */
  public onUpdated(
    handler: (record: {name}) => void,
    options?: AshSubscribeOptions & {{ id?: string; include?: {name}Include }},
  ): () => void {{
    const fields = build{name}SelectionSet(options?.include);
    const query = `subscription {name}Updated($id: ID) {{
      {updated}(id: $id) {{
        ${{fields}}
      }}
    }}`;
    return this.subscriptions.subscribe<{{ {updated}: {name} }}>(query, {{ id: options?.id }}, {{
      next: (data) => handler(data.{updated}),
      error: options?.onError,
      missed: options?.onMissed,
    }});
  }}

  /** Calls `handler` with the id of each record destroyed from now on, or only record `id`. */
  public onDestroyed(
    handler: (id: string) => void,
    options?: AshSubscribeOptions & {{ id?: string }},
  ): () => void {{
    const query = `subscription {name}Destroyed($id: ID) {{
      {destroyed}(id: $id)
    }}`;
    return this.subscriptions.subscribe<{{ {destroyed}: string }}>(query, {{ id: options?.id }}, {{
      next: (data) => handler(data.{destroyed}),
      error: options?.onError,
      missed: options?.onMissed,
    }});
  }}
"#
    )
}

/// `live()` for a resource's query builder.
pub fn generate_query_builder_live(res: &ResourceDef) -> String {
    let name = res.name;
    let pk = primary_key(res);
    format!(
        r#"
  /**
   * Keeps this query's results in sync with the server, calling `listener` with the
   * current list after every change. See `ashLiveQuery`.
   */
  public live(listener: (items: {name}[]) => void, options?: AshLiveOptions): AshLiveQuery {{
    const client = new {name}Client(this.transport, this.subscriptions);
    const include = this._include;
    return ashLiveQuery<{name}>(
      {{
        key: (record) => String(record.{pk}),
        fetch: () => this.all(),
        exact:
          !this._filter && this._sort.length === 0 && this._limit === undefined && this._offset === undefined,
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
  offset?: number;
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
    if (params?.offset !== undefined) builder.offset(params.offset);
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
