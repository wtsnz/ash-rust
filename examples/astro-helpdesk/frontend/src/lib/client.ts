import { createAshClient, type AshClient } from "./ash";

const API_BASE_URL =
  (typeof process !== "undefined" && process.env.API_URL) ||
  "http://127.0.0.1:4000";

export function getAshClient(baseUrl = API_BASE_URL): AshClient {
  return createAshClient({
    baseUrl,
    graphqlEndpoint: "/graphql",
  });
}

export const ash = getAshClient();
export * from "./ash";
