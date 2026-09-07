/**
 * Clerk client-token cache for the Tauri WebView (the "Expo pattern").
 *
 * ⚠️ RELIES ON CLERK INTERNAL API — `Clerk.__internal_onBeforeRequest` /
 * `__internal_onAfterResponse` from `@clerk/clerk-js`. These hooks are what
 * `@clerk/clerk-expo` uses for native token auth, but they are not public;
 * keep `@clerk/clerk-js`, `@clerk/react` and `@clerk/ui` pinned together
 * (package.json pins ^6 / ^6 / ^1) and re-verify on upgrades.
 *
 * Flow:
 *  - before every Frontend API request: drop cookies (`credentials: "omit"`),
 *    mark the request native (`_is_native=1`) and attach the cached client JWT
 *    as the `authorization` header.
 *  - after every response: persist the rotated JWT from the `authorization`
 *    response header. It is stored in the macOS keychain through
 *    `bluey.auth.storeToken` immediately and re-associated with the signed-in
 *    user via `bluey.auth.storeSession` once the user object is known.
 */

import type { Clerk } from "@clerk/clerk-js";

import { bluey } from "@/lib/tauri/api";
import type { AuthUser } from "@/lib/types";

let cachedToken: string | null = null;
let loadedFromStore = false;
/** Latest user mirrored by the AuthGate bridge; needed to persist the token. */
let knownUser: AuthUser | null = null;

async function loadToken(): Promise<string | null> {
  if (!loadedFromStore) {
    loadedFromStore = true;
    try {
      cachedToken = await bluey.auth.loadClientToken();
    } catch (error) {
      console.warn("[auth] failed to load stored client token", error);
    }
  }
  return cachedToken;
}

async function saveToken(token: string): Promise<void> {
  if (token === cachedToken) return;
  cachedToken = token;
  try {
    if (knownUser) {
      await bluey.auth.storeSession({ clientToken: token, user: knownUser });
    } else {
      // Rotated JWTs can arrive before the user object resolves — persist the token alone.
      await bluey.auth.storeToken({ clientToken: token });
    }
  } catch (error) {
    console.warn("[auth] failed to persist client token", error);
  }
}

/** Called by the AuthGate bridge whenever the Clerk user changes. */
export async function mirrorUser(user: AuthUser | null): Promise<void> {
  knownUser = user;
  if (user && cachedToken) {
    try {
      await bluey.auth.storeSession({ clientToken: cachedToken, user });
    } catch (error) {
      console.warn("[auth] failed to mirror session", error);
    }
  }
}

export async function clearTokenCache(): Promise<void> {
  cachedToken = null;
  knownUser = null;
  loadedFromStore = true;
  try {
    await bluey.auth.clearSession();
  } catch (error) {
    console.warn("[auth] failed to clear stored session", error);
  }
}

/** Install the native token cache hooks on a clerk-js instance (before `load`). */
export function installClerkTokenCache(clerk: Clerk): void {
  clerk.__internal_onBeforeRequest(async (requestInit) => {
    requestInit.credentials = "omit";
    requestInit.url?.searchParams.append("_is_native", "1");
    const headers = new Headers(requestInit.headers);
    headers.set("authorization", (await loadToken()) ?? "");
    requestInit.headers = headers;
  });

  clerk.__internal_onAfterResponse(async (_requestInit, response) => {
    const header = response?.headers?.get("authorization");
    if (header) await saveToken(header);
  });
}
