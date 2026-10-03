// Calls both desks through the AshTypescript client the Elixir desk generates
// (`ash_rpc.ts`), and compares what they answer: the generated client works against the
// ash-rust desk as it does against the Elixir one.
//
//   node client/smoke.ts --rust http://127.0.0.1:4701 --elixir http://127.0.0.1:4702 --fixture fixture.json
//
// Node runs the TypeScript as it is (Node 22.18 or later strips the types). Both desks
// must have loaded the same fixture; it writes to both alike.

import { readFileSync } from "node:fs";
import {
  getTicket,
  listAgents,
  listTickets,
  openTicket,
  routeTicket,
  startTicket,
  viewTicket,
} from "./ash_rpc.ts";

const ORG = "acme";

const flag = (name: string): string => {
  const at = process.argv.indexOf(name);
  const value = at >= 0 ? process.argv[at + 1] : undefined;
  if (!value) {
    console.error("usage: node client/smoke.ts --rust URL --elixir URL --fixture fixture.json");
    process.exit(2);
  }
  return value;
};

const desks = { rust: flag("--rust"), elixir: flag("--elixir") };
const fixture = JSON.parse(readFileSync(flag("--fixture"), "utf8"));

type Role = "admin" | "agent" | "viewer";
const staff = (role: Role): string =>
  fixture.agents.find((a: any) => a.org === ORG && a.role === role).id;
const ticket = (pick: (t: any) => boolean): string =>
  fixture.tickets.find((t: any) => t.org === ORG && pick(t)).id;

const open = ticket((t) => t.status === "open" && !t.confidential);
const secret = ticket((t) => t.status === "new" && t.confidential);

/** What the client needs to reach `base` as `role`: where to send, and who's asking. */
const as = (base: string, role: Role) => ({
  headers: { "x-org": ORG, "x-actor": staff(role), "x-role": role },
  customFetch: (input: RequestInfo | URL, init?: RequestInit) => fetch(new URL(String(input), base), init),
});

/** What's compared of a result: its data, or the types of its errors. */
const comparable = (result: any): unknown => {
  if (!result.success) return { errors: result.errors.map((e: any) => e.type) };
  const strip = (value: any): any =>
    Array.isArray(value)
      ? value.map(strip)
      : value && typeof value === "object"
        ? Object.fromEntries(
            Object.entries(value)
              .filter(([key]) => key !== "nextPage" && key !== "previousPage")
              .map(([key, v]) => [key, strip(v)]),
          )
        : value;
  return { data: strip(result.data) };
};

const calls: Array<[string, (base: string) => Promise<unknown>]> = [
  [
    "listTickets: filtered, sorted, a keyset page",
    (base) =>
      listTickets({
        ...as(base, "agent"),
        fields: ["id", "subject", "priority", "commentCount", { assignee: ["name"] }],
        filter: { priority: { greaterThanOrEqual: 3 } },
        sort: ["-insertedAt", "id"],
        page: { limit: 5 },
      }),
  ],
  [
    "getTicket: with relationships",
    (base) =>
      getTicket({
        ...as(base, "viewer"),
        getBy: { id: open },
        fields: ["id", "status", "requesterEmail", { tags: ["name"] }],
      }),
  ],
  [
    "listAgents",
    (base) => listAgents({ ...as(base, "admin"), fields: ["name", "role", "openAssigned"], sort: "name" }),
  ],
  ["viewTicket", (base) => viewTicket({ ...as(base, "agent"), identity: open, fields: ["viewCount", "version"] })],
  [
    "openTicket: with comments",
    (base) =>
      openTicket({
        ...as(base, "agent"),
        input: {
          subject: "Printer on fire",
          body: "Smoke everywhere",
          priority: 4,
          requesterEmail: "pat@example.com",
          comments: [{ body: "Called it in" }],
        },
        fields: ["subject", "status", "commentCount", { comments: ["body"] }],
      }),
  ],
  [
    "routeTicket",
    async (base) => {
      const routed: any = await routeTicket({
        ...as(base, "agent"),
        input: { subject: "Cannot log in", body: "Since Monday", priority: 2, requesterEmail: "sam@example.com" },
      });
      // Each desk's new ticket has its own id.
      return routed.success ? { success: true, data: typeof routed.data } : routed;
    },
  ],
  [
    "startTicket: one the viewer can't read",
    (base) => startTicket({ ...as(base, "viewer"), identity: secret, fields: ["status"] }),
  ],
];

let differed = 0;
for (const [name, call] of calls) {
  const [rust, elixir] = [comparable(await call(desks.rust)), comparable(await call(desks.elixir))];
  if (JSON.stringify(rust) === JSON.stringify(elixir)) {
    console.log(`  ok    ${name}`);
  } else {
    differed += 1;
    console.log(`  DIFF  ${name}\n          rust   ${JSON.stringify(rust)}\n          elixir ${JSON.stringify(elixir)}`);
  }
}
console.log(`\n${calls.length - differed} matched, ${differed} differed`);
process.exit(differed === 0 ? 0 : 1);
