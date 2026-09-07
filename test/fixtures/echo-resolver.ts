#!/usr/bin/env bun
import { serveResolver } from "../../src/resolver/server.ts";

serveResolver({
  describe() {
    return {
      name: "echo",
      version: "1",
      kinds: ["text-quote"],
      verifierKinds: [],
      tier: 3,
      advisory: true,
    };
  },
  resolve() {
    return { advisories: [{ resolver: "echo", message: "echo" }] };
  },
});
