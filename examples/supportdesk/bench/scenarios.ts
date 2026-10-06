// What the benchmark asks of the desks: each scenario as RPC, through the generated
// AshTypescript client, and as GraphQL, or as JSON where the desks serve it only so.
// Every request is drawn from a seeded generator, so both desks are asked the same
// things in the same order.

import {
  getTicket,
  listTickets,
  openTicket,
  resolveTicket,
  routeTicket,
  startTicket,
  viewTicket,
} from "../client/ash_rpc.ts";

export type Role = "admin" | "agent" | "viewer";
export type Transport = "rpc" | "graphql" | "json";

/** The fixture, as much of it as the requests need. */
export type World = {
  orgs: Array<{
    slug: string;
    admin: string;
    agents: string[];
    viewer: string;
    /** Tickets anyone in the org may read. */
    readable: string[];
  }>;
};

export const world = (fixture: any): World => ({
  orgs: fixture.orgs.map((org: any) => {
    const staff = fixture.agents.filter((a: any) => a.org === org.slug && a.active);
    const of = (role: Role) => staff.filter((a: any) => a.role === role).map((a: any) => a.id);
    return {
      slug: org.slug,
      admin: of("admin")[0],
      agents: of("agent"),
      viewer: of("viewer")[0],
      readable: fixture.tickets.filter((t: any) => t.org === org.slug && !t.confidential).map((t: any) => t.id),
    };
  }),
});

/** A seeded generator (mulberry32): the same seed asks the same things. */
export const rng = (seed: number) => {
  let a = seed >>> 0;
  const next = () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  return { next, pick: <T>(items: T[]): T => items[Math.floor(next() * items.length)] };
};
export type Rng = ReturnType<typeof rng>;

export type Who = { org: string; role: Role; id: string };

/** Someone of a random org, in a random role: admins, agents and viewers alike. */
export const anyone = (w: World, r: Rng): Who => {
  const org = r.pick(w.orgs);
  const role = r.pick<Role>(["admin", "agent", "viewer"]);
  const id = role === "admin" ? org.admin : role === "viewer" ? org.viewer : r.pick(org.agents);
  return { org: org.slug, role, id };
};

/** Someone who may write: an agent or an admin. */
export const writer = (w: World, r: Rng, org = r.pick(w.orgs)): Who =>
  r.next() < 0.2 ? { org: org.slug, role: "admin", id: org.admin } : { org: org.slug, role: "agent", id: r.pick(org.agents) };

const headers = (who: Who) => ({ "x-org": who.org, "x-actor": who.id, "x-role": who.role });

/** What the generated client needs to reach `base` as `who`. */
const as = (base: string, who: Who) => ({
  headers: headers(who),
  customFetch: (input: RequestInfo | URL, init?: RequestInit) => fetch(base + String(input), init),
});

export const post = async (base: string, path: string, who: Who, body: unknown, signal?: AbortSignal): Promise<{ status: number; body: any }> => {
  const response = await fetch(base + path, {
    method: "POST",
    headers: { "content-type": "application/json", ...headers(who) },
    body: JSON.stringify(body),
    signal,
  });
  return { status: response.status, body: await response.json() };
};

export const graphql = async (base: string, who: Who, query: string, variables: object = {}, signal?: AbortSignal) =>
  (await post(base, "/graphql", who, { query, variables }, signal)).body;

/** A GraphQL answer failed: no data, or an error but a redacted field's. */
const gqlFailed = (body: any, mutation?: string): boolean =>
  !body?.data ||
  (body.errors ?? []).some((e: any) => e.code !== "forbidden_field" && e.extensions?.code !== "forbidden_field") ||
  (mutation !== undefined && (body.data[mutation]?.errors ?? []).length > 0);

/** What one request answered: whether it succeeded, and what's compared of it. */
export type Outcome = { ok: boolean; body: unknown };

/** `signal` aborts a request the driver has given up on (the saturation scenarios set one). */
export type Op = (base: string, r: Rng, w: World, signal?: AbortSignal) => Promise<Outcome>;

export type Scenario = {
  name: string;
  /** Reads run closed loop, many clients each asking again as soon as answered. Writes
   *  run open loop, at a fixed rate, so both desks do the same work. */
  tier: "read" | "write" | "saturation";
  what: string;
  ops: Partial<Record<Transport, Op>>;
  /** Saturation: the scenarios `saturation.ts` runs together, a stream of each class of
   *  request at once, which `bench.ts` doesn't run. Writes: requests a second (each op may
   *  make several). */
  rate?: number;
};

// Reads ------------------------------------------------------------------------------

const INBOX_GQL = `query {
  listTickets(first: 25, sort: [{ field: INSERTED_AT, order: DESC }, { field: ID }]) {
    count results { id subject status priority confidential requesterEmail assigneeId assignee { name } }
  }
}`;

const DASHBOARD_GQL = `query {
  listTickets(
    first: 25,
    filter: { commentCount: { greaterThanOrEqual: 5 }, weight: { greaterThanOrEqual: 30 } },
    sort: [{ field: COMMENT_COUNT, order: DESC }, { field: INSERTED_AT, order: DESC }, { field: ID }]
  ) { count results { id commentCount publicCommentCount hasInternalNotes weight subjectLength } }
}`;

const DETAIL_GQL = `query T($id: ID!) {
  getTicket(id: $id) {
    id subject status commentCount author { name } assignee { name }
    tags(sort: [{ field: NAME }]) { name }
    comments(sort: [{ field: INSERTED_AT, order: DESC }, { field: ID }], limit: 3) { body internal }
  }
}`;

/** An RPC answer, but for keyset cursors, which each desk encodes its own way. */
const rpcOutcome = (result: any): Outcome => {
  if (!result.success) return { ok: false, body: { errors: result.errors.map((e: any) => e.type) } };
  const { nextPage, previousPage, after, before, ...data } = result.data ?? {};
  return { ok: true, body: Array.isArray(result.data) ? result.data : data };
};

const gqlOutcome = (body: any, mutation?: string): Outcome => ({
  ok: !gqlFailed(body, mutation),
  body: { data: body?.data ?? null, errors: (body?.errors ?? []).map((e: any) => e.path).sort() },
});

const SAT_CHEAP_GQL = `query T($id: ID!) { getTicket(id: $id) { id subject status priority } }`;

const SAT_HEAVY_GQL = `query {
  listTickets(first: 250, sort: [{ field: INSERTED_AT, order: DESC }, { field: ID }]) {
    count
    results {
      id subject body status priority confidential requesterEmail assigneeId authorId viewCount
      commentCount publicCommentCount hasInternalNotes weight subjectLength
      assignee { name email }
      author { name }
      tags(sort: [{ field: NAME }]) { name }
      comments(sort: [{ field: INSERTED_AT, order: DESC }, { field: ID }], limit: 5) { body internal authorId }
    }
  }
}`;

const nobody: Who = { org: "", role: "viewer", id: "" };

const SAT_CPU_HEAVY_GQL = `query {
  listTickets(first: 25, sort: [{ field: TITLE, order: DESC }], filter: { title: { contains: "a" } }) {
    count
    results { id title status priority }
  }
}`;

export const scenarios: Scenario[] = [
  {
    name: "inbox",
    tier: "read",
    what: "the newest tickets, a keyset page with a count, as an admin, agent or viewer of any org: policies, a field policy (requester emails), multitenancy",
    ops: {
      rpc: async (base, r, w) =>
        rpcOutcome(
          await listTickets({
            ...as(base, anyone(w, r)),
            fields: ["id", "subject", "status", "priority", "confidential", "requesterEmail", "assigneeId", { assignee: ["name"] }],
            sort: "-insertedAt,id",
            page: { limit: 25, count: true },
          } as any),
        ),
      graphql: async (base, r, w) => gqlOutcome(await graphql(base, anyone(w, r), INBOX_GQL)),
    },
  },
  {
    name: "dashboard",
    tier: "read",
    what: "tickets filtered and sorted by aggregates (comment counts) and a calculation (weight), with a count",
    ops: {
      rpc: async (base, r, w) =>
        rpcOutcome(
          await listTickets({
            ...as(base, anyone(w, r)),
            fields: ["id", "commentCount", "publicCommentCount", "hasInternalNotes", "weight", "subjectLength"],
            filter: { commentCount: { greaterThanOrEqual: 5 }, weight: { greaterThanOrEqual: 30 } },
            sort: "-commentCount,-insertedAt,id",
            page: { limit: 25, count: true },
          } as any),
        ),
      graphql: async (base, r, w) => gqlOutcome(await graphql(base, anyone(w, r), DASHBOARD_GQL)),
    },
  },
  {
    name: "detail",
    tier: "read",
    what: "one ticket with its author, assignee and tags, and over GraphQL its three newest comments: nested relationships. (Not over RPC: AshTypescript 0.19 reads a relationship selected with options without the actor, so the Elixir desk would read no comments; see GAPS.md)",
    ops: {
      rpc: async (base, r, w) => {
        const who = anyone(w, r);
        const id = r.pick(w.orgs.find((o) => o.slug === who.org)!.readable);
        return rpcOutcome(
          await getTicket({
            ...as(base, who),
            getBy: { id },
            fields: [
              "id",
              "subject",
              "status",
              "commentCount",
              { author: ["name"] },
              { assignee: ["name"] },
              { tags: { fields: ["name"], sort: "name" } },
            ],
          } as any),
        );
      },
      graphql: async (base, r, w) => {
        const who = anyone(w, r);
        const id = r.pick(w.orgs.find((o) => o.slug === who.org)!.readable);
        return gqlOutcome(await graphql(base, who, DETAIL_GQL, { id }));
      },
    },
  },

  // Writes ---------------------------------------------------------------------------

  {
    name: "workflow",
    tier: "write",
    rate: 20,
    what: "open a ticket with two comments (managed relationship, validations), then start and resolve it (state machine): three requests",
    ops: {
      rpc: async (base, r, w) => {
        const who = writer(w, r);
        const opened: any = await openTicket({
          ...as(base, who),
          input: {
            subject: `Printer on fire ${Math.floor(r.next() * 1e6)}`,
            body: "Smoke everywhere",
            priority: 1 + Math.floor(r.next() * 4),
            requesterEmail: "pat@example.com",
            comments: [{ body: "Called it in" }, { body: "Escalating", internal: true }],
          },
          fields: ["id"],
        });
        if (!opened.success) return rpcOutcome(opened);
        const started = await startTicket({ ...as(base, who), identity: opened.data.id, fields: ["status"] });
        if (!started.success) return rpcOutcome(started);
        return rpcOutcome(await resolveTicket({ ...as(base, who), identity: opened.data.id, fields: ["status"] }));
      },
      graphql: async (base, r, w) => {
        const who = writer(w, r);
        const input = {
          subject: `Printer on fire ${Math.floor(r.next() * 1e6)}`,
          body: "Smoke everywhere",
          priority: 1 + Math.floor(r.next() * 4),
          requesterEmail: "pat@example.com",
          comments: [{ body: "Called it in" }, { body: "Escalating", internal: true }],
        };
        const opened = await graphql(
          base,
          who,
          "mutation O($input: OpenTicketInput!) { openTicket(input: $input) { result { id } errors { code } } }",
          { input },
        );
        if (gqlFailed(opened, "openTicket")) return gqlOutcome(opened, "openTicket");
        const id = opened.data.openTicket.result.id;
        const started = await graphql(base, who, "mutation S($id: ID!) { startTicket(id: $id) { result { status } errors { code } } }", { id });
        if (gqlFailed(started, "startTicket")) return gqlOutcome(started, "startTicket");
        return gqlOutcome(
          await graphql(base, who, "mutation R($id: ID!) { resolveTicket(id: $id) { result { status } errors { code } } }", { id }),
          "resolveTicket",
        );
      },
    },
  },
  {
    name: "route",
    tier: "write",
    rate: 20,
    what: "the route generic action: in one transaction, open a ticket with a comment, assign it to the least-loaded agent, and record an audit event. (Over GraphQL the comment goes as JSON text, as AshGraphql takes a map; ash-graphql takes that or an object)",
    ops: {
      rpc: async (base, r, w) => {
        const routed: any = await routeTicket({
          ...as(base, writer(w, r)),
          input: {
            subject: `Cannot log in ${Math.floor(r.next() * 1e6)}`,
            body: "Since Monday",
            priority: 1 + Math.floor(r.next() * 4),
            requesterEmail: "sam@example.com",
            comments: [{ body: "Tried a reset" }],
          },
        });
        return { ok: routed.success, body: routed.success ? "routed" : routed.errors };
      },
      graphql: async (base, r, w) => {
        const input = {
          subject: `Cannot log in ${Math.floor(r.next() * 1e6)}`,
          body: "Since Monday",
          priority: 1 + Math.floor(r.next() * 4),
          requesterEmail: "sam@example.com",
          comments: [JSON.stringify({ body: "Tried a reset" })],
        };
        const body = await graphql(base, writer(w, r), "mutation R($input: RouteTicketInput!) { routeTicket(input: $input) }", { input });
        return { ok: !gqlFailed(body), body: body.errors ?? "routed" };
      },
    },
  },
  {
    name: "counters",
    tier: "write",
    rate: 100,
    what: "view one of ten hot tickets: an atomic increment under contention, under the ticket's optimistic lock (checked after: every acknowledged view counted). Each view reads the ticket and writes at the version it read, so one can lose a race, as not found, on either desk",
    ops: {
      rpc: async (base, r, w) => {
        const id = hot(w)[Math.floor(r.next() * 10)];
        return rpcOutcome(await viewTicket({ ...as(base, writer(w, r, w.orgs[0])), identity: id, fields: ["viewCount"] }));
      },
      graphql: async (base, r, w) => {
        const id = hot(w)[Math.floor(r.next() * 10)];
        return gqlOutcome(
          await graphql(base, writer(w, r, w.orgs[0]), "mutation V($id: ID!) { viewTicket(id: $id) { result { viewCount } errors { code } } }", { id }),
          "viewTicket",
        );
      },
    },
  },
  {
    name: "edit races",
    tier: "write",
    rate: 10,
    what: "two edits of a ticket at the version both read, at once: optimistic locking (checked: exactly one wins, the other is stale)",
    ops: {
      json: async (base, r, w) => {
        const org = w.orgs[1 + Math.floor(r.next() * (w.orgs.length - 1))];
        const who = writer(w, r, org);
        const id = r.pick(org.readable);
        const read = await graphql(base, who, "query T($id: ID!) { getTicket(id: $id) { version } }", { id });
        const version = read?.data?.getTicket?.version;
        if (version === undefined) return { ok: false, body: read };
        const edits = await Promise.all(
          [1, 2].map((priority) => post(base, "/api/edit", who, { id, version, priority })),
        );
        const statuses = edits.map((e) => e.status).sort();
        return { ok: statuses[0] === 200 && statuses[1] === 409, body: statuses };
      },
    },
  },
  {
    name: "bulk",
    tier: "write",
    rate: 2,
    what: "create 100 tickets, assign them all, then destroy them, each a bulk action in batches",
    ops: {
      json: async (base, r, w) => {
        const org = r.pick(w.orgs);
        const result = await post(base, "/api/bulk", { org: org.slug, role: "admin", id: org.admin }, { count: 100, assigneeId: org.agents[0] });
        return { ok: result.status === 200 && result.body.destroyed === 100, body: result.body };
      },
    },
  },
  // Saturation: what `saturation.ts` runs, a stream of each at once ----------------------
  {
    name: "sat-cheap",
    tier: "saturation",
    what: "one ticket by id over GraphQL, as a viewer of any org: a small request that should stay quick whatever else the desk is doing",
    ops: {
      graphql: async (base, r, w, signal) => {
        const org = r.pick(w.orgs);
        const who: Who = { org: org.slug, role: "viewer", id: org.viewer };
        const body = await graphql(base, who, SAT_CHEAP_GQL, { id: r.pick(org.readable) }, signal);
        return { ok: !gqlFailed(body) && !!body.data.getTicket, body };
      },
    },
  },
  {
    name: "sat-heavy",
    tier: "saturation",
    what: "the 250 newest tickets of an org with their assignee, author, tags, aggregates and five comments each, as its admin: a large page the desk must read, shape and serialize",
    ops: {
      graphql: async (base, r, w, signal) => {
        const org = r.pick(w.orgs);
        const who: Who = { org: org.slug, role: "admin", id: org.admin };
        const body = await graphql(base, who, SAT_HEAVY_GQL, {}, signal);
        return { ok: !gqlFailed(body) && (body.data.listTickets?.results?.length ?? 0) > 0, body };
      },
    },
  },
  // CPU-bound saturation, on the in-memory astro-helpdesk twins (`--target astro`) --------
  {
    name: "sat-cpu-cheap",
    tier: "saturation",
    what: "`{ __typename }` over GraphQL: the smallest request the GraphQL layer answers, with no data, so what it measures is how soon a worker gets to it",
    ops: {
      graphql: async (base, _r, _w, signal) => {
        const body = await graphql(base, nobody, "query { __typename }", {}, signal);
        return { ok: body?.data?.__typename === "RootQueryType", body };
      },
    },
  },
  {
    name: "sat-cpu-heavy",
    tier: "saturation",
    what: "tickets whose title contains a letter, sorted by title, 25 of them with the count: the desk filters, sorts and counts every ticket it holds in memory, and answers a small page",
    ops: {
      graphql: async (base, _r, _w, signal) => {
        const body = await graphql(base, nobody, SAT_CPU_HEAVY_GQL, {}, signal);
        return { ok: (body?.data?.listTickets?.count ?? 0) > 0 && !body.errors, body };
      },
    },
  },
];

/** The ten tickets `counters` contends on: the first org's first readable ones. */
export const hot = (w: World): string[] => w.orgs[0].readable.slice(0, 10);
