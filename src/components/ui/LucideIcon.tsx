import { icons, Sparkles, type LucideProps } from "lucide-react";

export interface LucideIconProps extends LucideProps {
  /** Kebab-case lucide name stored on modes, e.g. "graduation-cap". */
  name: string;
}

function toPascal(name: string): string {
  return name
    .split("-")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join("");
}

/** Renders a lucide icon by its kebab-case name with a safe fallback. */
export function LucideIcon({ name, ...props }: LucideIconProps) {
  const Icon = (icons as Record<string, (typeof icons)[keyof typeof icons]>)[toPascal(name)] ?? Sparkles;
  return <Icon {...props} />;
}
