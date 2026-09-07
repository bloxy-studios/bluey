import { cn } from "@/lib/utils/cn";

export interface SectionHeaderProps {
  title: string;
  description?: string;
  className?: string;
}

/** Settings section header: 15px semibold + 13px muted description. */
export function SectionHeader({ title, description, className }: SectionHeaderProps) {
  return (
    <header className={cn("mt-6 mb-3 first:mt-0", className)}>
      <h2 className="text-[15px] font-semibold text-fg">{title}</h2>
      {description ? <p className="mt-0.5 text-[13px] text-fg-muted">{description}</p> : null}
    </header>
  );
}
