import { Clerk } from "@clerk/clerk-js";

import { installClerkTokenCache } from "./token-cache";

let instance: Clerk | null = null;

/**
 * Singleton bundled clerk-js instance with the native token cache installed
 * before the provider loads it. Passed to `<ClerkProvider Clerk={...}>` so
 * nothing is hot-loaded from Clerk's CDN.
 */
export function getClerkInstance(publishableKey: string): Clerk {
  if (!instance) {
    instance = new Clerk(publishableKey);
    installClerkTokenCache(instance);
  }
  return instance;
}
