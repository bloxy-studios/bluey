/**
 * Authentication contract (ADR 0008). Sign-in happens in the system browser
 * (Clerk as the OAuth 2.0 / OpenID Connect provider); Rust exchanges the code,
 * keeps the tokens in the Keychain and only ever shares this status with the
 * WebView. Mirrors `bluey_core::types::auth`.
 */

export type AuthState = "unknown" | "signed_out" | "signed_in";

export interface AuthUser {
  id: string;
  email?: string;
  firstName?: string;
  lastName?: string;
  imageUrl?: string;
}

export interface AuthStatus {
  state: AuthState;
  user?: AuthUser;
  /** Whether OAuth tokens exist in secure storage. */
  hasStoredSession: boolean;
  /** Sign-in is configured (Clerk issuer + OAuth client id); otherwise Bluey runs without an account. */
  configured: boolean;
  /** A browser sign-in was started and Bluey is waiting for the redirect. */
  signInPending: boolean;
  checkedAt: string;
}

/** How the browser hands the authorization code back to Bluey. */
export type SignInRedirect = "deep_link" | "loopback";

/** What `auth_begin_sign_in` returns after opening the browser. */
export interface SignInStart {
  /** The authorization URL that was opened (for "copy link"). */
  url: string;
  redirect: SignInRedirect;
  /** When the pending flow expires (ISO 8601). */
  expiresAt: string;
}
