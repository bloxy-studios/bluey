import { FileText, LifeBuoy, Mail, CircleHelp, ExternalLink } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { Card } from "@/components/ui/Card";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { SettingRow } from "@/components/ui/SettingRow";
import { bluey } from "@/lib/tauri/api";
import type { DevInfo } from "@/lib/types";
import { openExternal } from "@/lib/utils/open-external";

const RELEASE_NOTES = [
  "AI chat responds faster, with screen context assembled natively.",
  "Long sessions keep better context, including earlier details that matter later.",
  "Modes can attach files so answers stay grounded in your documents.",
];

export default function AboutTab() {
  const [info, setInfo] = useState<DevInfo | null>(null);

  useEffect(() => {
    let alive = true;
    void bluey.app
      .getDevInfo()
      .then((i) => {
        if (alive) setInfo(i);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  return (
    <>
      <SectionHeader title="About" description="Release notes, support, and app information" />

      <Card className="mb-4">
        <div className="flex items-start gap-4">
          <div className="flex size-12 shrink-0 items-center justify-center rounded-card bg-bg-tile">
            <FileText className="size-5 text-fg-muted" strokeWidth={1.8} aria-hidden />
          </div>
          <div>
            <div className="flex items-baseline gap-2.5">
              <h3 className="text-[14px] font-semibold text-fg">Welcome to Bluey {info?.version ?? ""}</h3>
              <span className="text-[12.5px] text-fg-subtle">September 2026</span>
            </div>
            <p className="mt-1 text-[13px] text-fg-muted">
              Bluey is a real-time desktop copilot that understands your screen and your conversations.
            </p>
            <ul className="mt-2.5 flex list-disc flex-col gap-1 pl-4 text-[13px] text-fg-muted">
              {RELEASE_NOTES.map((note) => (
                <li key={note}>{note}</li>
              ))}
            </ul>
          </div>
        </div>
      </Card>

      <SettingRow icon={LifeBuoy} title="Help Center" description="Find answers and setup help.">
        <Button variant="secondary" size="sm" onClick={() => void openExternal("https://bluey.app/help")}>
          Open <ExternalLink className="size-3.5" aria-hidden />
        </Button>
      </SettingRow>

      <SettingRow icon={Mail} title="Contact Support" description="Reach the Bluey team directly.">
        <Button variant="secondary" size="sm" onClick={() => void openExternal("mailto:support@bluey.app")}>
          Email <ExternalLink className="size-3.5" aria-hidden />
        </Button>
      </SettingRow>

      <SettingRow icon={CircleHelp} title="Bluey Version" description="The desktop version currently installed.">
        <span className="text-[13px] text-fg-muted">
          {info ? `${info.version}${info.mockTransport ? " (mock)" : ""}` : "–"}
        </span>
      </SettingRow>
    </>
  );
}
