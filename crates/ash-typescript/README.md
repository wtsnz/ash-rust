# ash-typescript

`ash-typescript` generates **end-to-end type-safe TypeScript definitions, Zod validation schemas, and an isomorphic client SDK** directly from your declarative `ash-rust` domain and resource definitions.

It bridges the gap between your Rust backend and any modern TypeScript frontend—whether building **Server-Side Rendered (SSR)** apps in Next.js, Remix, or Astro, or **Single-Page Applications (SPA)** in React, Vite, Vue, or Svelte.

---

## Features

- 🔒 **End-to-End Type Safety**: Generates TypeScript interfaces for all resources, relationships, calculations, filters, and sort inputs.
- ⚡ **Zod Form Validation**: Emits ready-to-use Zod schemas mirroring your Ash validations (`string_length`, `numericality`, `one_of`, `present`). Feed them straight into React Hook Form or TanStack Form with 0 drift from backend rules.
- 🌐 **Isomorphic Zero-Dependency Client**: Standard `fetch` transport compatible with Node.js 18+, Bun, Deno, Next.js Server Components, Cloudflare Workers, and all modern browsers.
- 🚀 **Next.js & SSR Ready**: Dynamic per-request headers and cookies for Server Components and Server Actions without leaking auth state between requests.
- ⚛️ **TanStack Query (React Query) Integration**: First-class `queryOptions()` support for seamless prefetching on the server and hydration on the client.
- 🛠️ **CLI Integration**: Run `cargo ash codegen ts --out ./frontend/src/ash.ts` to keep frontend and backend in lockstep.

---

## Quick Start

### 1. Generating via Rust Code

You can generate your TypeScript SDK inside your Rust build script, test suite, or CLI:

```rust
use ash_typescript::{TypeScriptConfig, TypeScriptGenerator};
use my_app::DOMAIN;

let ts_code = TypeScriptGenerator::new()
    .config(
        TypeScriptConfig::new()
            .with_zod(true)
            .with_client(true)
            .with_react(true)
            .with_client_name("AshClient")
            .with_graphql_endpoint("/graphql"),
    )
    .add_domain(&DOMAIN)
    .generate_consolidated()?;

std::fs::write("./frontend/src/ash.ts", ts_code)?;
```

### 2. Generating via `cargo-ash` CLI

```bash
# Dump latest schema snapshots from database or resources
cargo ash schema dump

# Generate the complete TypeScript SDK
cargo ash codegen ts --out ./frontend/src/ash.ts
# or using the shortcut:
cargo ash ts -o ./frontend/src/ash.ts
```

---

## Generated Code Breakdown

### 1. Type Definitions & Filter Inputs

```typescript
export interface Ticket {
  id: string;
  title: string;
  status: "open" | "in_progress" | "closed";
  priority: number;
  author_id?: string | null;
  author?: User | null;
}

export interface TicketFilterInput {
  id?: UuidFilter;
  title?: StringFilter;
  status?: StringFilter;
  priority?: IntFilter;
  and?: TicketFilterInput[];
  or?: TicketFilterInput[];
  not?: TicketFilterInput;
}
```

### 2. Zod Validation Schemas

Matches your Ash action validations for frontend forms:

```typescript
import { z } from "zod";

export const TicketOpenInputSchema = z.object({
  title: z.string().min(5).max(255),
  status: z.enum(["open", "in_progress", "closed"]),
  priority: z.number().int().min(1).max(5),
  author_id: z.string().uuid().nullable().optional(),
});

export type TicketOpenInput = z.infer<typeof TicketOpenInputSchema>;
```

Usage with React Hook Form:

```tsx
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { TicketOpenInputSchema, TicketOpenInput } from "@/ash";

export function TicketForm() {
  const { register, handleSubmit, formState: { errors } } = useForm<TicketOpenInput>({
    resolver: zodResolver(TicketOpenInputSchema),
  });

  const onSubmit = async (data: TicketOpenInput) => {
    await ash.ticket.open(data);
  };

  return (
    <form onSubmit={handleSubmit(onSubmit)}>
      <input {...register("title")} />
      {errors.title && <p>{errors.title.message}</p>}
      <button type="submit">Submit</button>
    </form>
  );
}
```

---

## Architectural Patterns: SSR vs SPA

### Pattern A: Next.js Server Components & Server Actions (SSR)

In SSR frameworks, tokens and cookies must be resolved dynamically per incoming HTTP request:

```typescript
// lib/ash.server.ts
import { cookies } from "next/headers";
import { createAshClient } from "@/ash";

export async function getServerAshClient() {
  const cookieStore = await cookies();
  const token = cookieStore.get("ash_token")?.value;

  return createAshClient({
    baseUrl: process.env.API_URL || "http://localhost:4000",
    headers: () => ({
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    }),
  });
}
```

Fetching in a Server Component:

```tsx
// app/tickets/page.tsx
import { getServerAshClient } from "@/lib/ash.server";

export default async function TicketsPage() {
  const ash = await getServerAshClient();
  const tickets = await ash.ticket.query()
    .filter({ status: { eq: "open" } })
    .sort("priority", "desc")
    .include({ author: true })
    .all();

  return (
    <ul>
      {tickets.map((t) => (
        <li key={t.id}>{t.title} - {t.author?.name}</li>
      ))}
    </ul>
  );
}
```

Mutating in a Next.js Server Action:

```typescript
// app/tickets/actions.ts
"use server";
import { getServerAshClient } from "@/lib/ash.server";
import { TicketOpenInput } from "@/ash";

export async function createTicketAction(input: TicketOpenInput) {
  const ash = await getServerAshClient();
  return await ash.ticket.open(input);
}
```

---

### Pattern B: Single-Page Application (SPA) / Vite / React

In client-side single page apps, you can initialize a shared client singleton:

```typescript
// lib/ash.client.ts
import { createAshClient } from "@/ash";

export const ash = createAshClient({
  baseUrl: import.meta.env.VITE_API_URL || "http://localhost:4000",
  headers: () => {
    const token = localStorage.getItem("token");
    return token ? { Authorization: `Bearer ${token}` } : {};
  },
});
```

Using TanStack Query v5 with `queryOptions()`:

```tsx
import { useQuery } from "@tanstack/react-query";
import { ash } from "@/lib/ash.client";

export function TicketList() {
  const { data: tickets, isLoading } = useQuery(
    ash.ticket.query()
      .filter({ status: { eq: "open" } })
      .sort("priority", "desc")
      .include({ author: true })
      .queryOptions()
  );

  if (isLoading) return <div>Loading...</div>;

  return (
    <div>
      {tickets?.map((t) => (
        <div key={t.id}>{t.title}</div>
      ))}
    </div>
  );
}
```

---

## Relay Keyset Pagination

Ash provides built-in keyset pagination matching Relay specifications:

```typescript
const page = await ash.ticket.query()
  .filter({ status: { eq: "open" } })
  .page(20, endCursor);

console.log(page.results);
console.log(page.pageInfo.hasNextPage);
console.log(page.pageInfo.endCursor);
```

---

## License

MIT
