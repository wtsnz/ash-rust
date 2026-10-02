import { CommandClient } from "./ash";

/** The command center's API: `PUBLIC_API_URL`, or the local server `cargo run -p cybercab` starts. */
export const API_URL: string = import.meta.env.PUBLIC_API_URL ?? "http://127.0.0.1:4000";

export const client = new CommandClient({ baseUrl: API_URL });

export * from "./ash";
