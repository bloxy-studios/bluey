import { ClerkProvider, useAuth, useUser } from "@clerk/react";
import { ui as clerkUi } from "@clerk/ui";
import { dark } from "@clerk/ui/themes";
import { useEffect, type ReactNode } from "react";

import { bluey } from "@/lib/tauri/api";
import { getTransport } from "@/lib/tauri/transport";
import type { AuthUser } from "@/lib/types";
import { clerkPublishableKey, DEV_USER, resolveAuthMode, useAuthStore } from "./auth-store";
import { getClerkInstance } from "./clerk-instance";
import { clearTokenCache, mirrorUser } from "./token-cache";

/** Clerk appearance mapped onto the Bluey design tokens. */
const APPEARANCE = {
  theme: dark,
  variables: {
    colorPrimary: "#0a84ff",
    colorPrimaryForeground: "#ffffff",
    colorBackground: "#161616",
    colorForeground: "#f2f2f2",
    colorMutedForeground: "#9a9a9a",
    colorInput: "#1e1e1e",
    colorInputForeground: "#f2f2f2",
    colorBorder: "#262626",
    colorDanger: "#ff453a",
    colorSuccess: "#30d158",
    borderRadius: "10px",
    fontFamily: '-apple-system, "SF Pro Text", Inter, system-ui, sans-serif',
  },
} as const;

/** Mirrors Clerk auth state into the auth store + secure storage. */
function ClerkAuthBridge() {
  const { isLoaded, isSignedIn } = useAuth();
  const { user } = useUser();

  useEffect(() => {
    const store = useAuthStore.getState();
    if (!isLoaded) {
      store.set({ state: "unknown" });
      return;
    }
    if (isSignedIn && user) {
      const authUser: AuthUser = {
        id: user.id,
        email: user.primaryEmailAddress?.emailAddress,
        firstName: user.firstName ?? undefined,
        lastName: user.lastName ?? undefined,
        imageUrl: user.imageUrl,
      };
      store.set({ state: "signed_in", user: authUser });
      void mirrorUser(authUser);
    } else {
      const wasSignedIn = useAuthStore.getState().state === "signed_in";
      store.set({ state: "signed_out", user: null });
      void mirrorUser(null);
      if (wasSignedIn) void clearTokenCache();
    }
  }, [isLoaded, isSignedIn, user]);

  return null;
}

export interface ClerkRootProps {
  children: ReactNode;
}

/**
 * Auth provider root. With a publishable key it mounts the bundled Clerk
 * (clerk-js instance + @clerk/ui, native token cache); without one it falls
 * back to a signed-in developer identity (mock transport) or the
 * "unconfigured" state handled by AuthGate.
 */
export function ClerkRoot({ children }: ClerkRootProps) {
  const publishableKey = clerkPublishableKey();
  const mode = resolveAuthMode(publishableKey, getTransport().kind);

  useEffect(() => {
    const store = useAuthStore.getState();
    if (mode === "dev") {
      store.set({ mode, state: "signed_in", user: DEV_USER });
      void bluey.auth.storeSession({ clientToken: "dev-token", user: DEV_USER }).catch(() => undefined);
    } else if (mode === "unconfigured") {
      store.set({ mode, state: "signed_out", user: null });
    } else {
      store.set({ mode });
    }
  }, [mode]);

  if (mode !== "clerk" || !publishableKey) {
    return <>{children}</>;
  }

  return (
    <ClerkProvider
      publishableKey={publishableKey}
      Clerk={getClerkInstance(publishableKey)}
      standardBrowser={false}
      allowedRedirectProtocols={["tauri:"]}
      ui={clerkUi}
      appearance={APPEARANCE}
    >
      <ClerkAuthBridge />
      {children}
    </ClerkProvider>
  );
}
