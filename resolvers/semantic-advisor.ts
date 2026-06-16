#!/usr/bin/env bun
/**
 * Out-of-process entry point for the Tier-3 semantic advisor (§7.4). Register it
 * in `.claims/resolvers.json`:
 *
 *   { "resolvers": [
 *       { "name": "semantic-advisor",
 *         "command": "bun",
 *         "args": ["run", "resolvers/semantic-advisor.ts"] }
 *   ] }
 */
import { serveResolver } from "../src/resolver/server.ts";
import { semanticAdvisorHandler } from "../src/resolver/builtin/semantic-advisor.ts";

serveResolver(semanticAdvisorHandler());
