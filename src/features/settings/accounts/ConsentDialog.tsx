import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import type { ProviderCopy } from "./account-copy";

export interface ConsentDialogProps {
  provider: ProviderCopy | null;
  onCancel: () => void;
  /** The user read the four facts and wants to continue to the browser. */
  onAccept: (provider: ProviderCopy) => void;
}

/**
 * Shown once per provider before the browser opens (ADR 0009): what is sent,
 * whose plan limits are used, that the integration is unofficial, and what
 * Bluey does when it stops working. Plain language, no legal boilerplate.
 */
export function ConsentDialog({ provider, onCancel, onAccept }: ConsentDialogProps) {
  return (
    <Dialog
      open={provider !== null}
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
      title={provider?.consent.title ?? ""}
      footer={
        provider ? (
          <>
            <Button variant="secondary" onClick={onCancel}>
              Cancel
            </Button>
            <Button variant="primary" onClick={() => onAccept(provider)}>
              {provider.consent.confirm}
            </Button>
          </>
        ) : null
      }
    >
      {provider ? (
        <div className="flex flex-col gap-3 text-[13px] leading-relaxed text-fg-muted" data-testid="account-consent">
          {provider.consent.paragraphs.map((paragraph) => (
            <p key={paragraph.slice(0, 40)}>{paragraph}</p>
          ))}
        </div>
      ) : null}
    </Dialog>
  );
}
