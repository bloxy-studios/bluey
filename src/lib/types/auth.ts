/** Authentication contract. Clerk runs in the WebView; Rust stores the client token securely. */

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
  /** Whether a persisted client token exists in secure storage. */
  hasStoredSession: boolean;
  checkedAt: string;
}
