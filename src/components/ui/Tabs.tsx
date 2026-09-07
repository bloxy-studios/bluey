import { cn } from "@/lib/utils/cn";

export interface TabOption<T extends string> {
  value: T;
  label: string;
}

export interface SegmentedTabsProps<T extends string> {
  value: T;
  onValueChange: (value: T) => void;
  options: Array<TabOption<T>>;
  className?: string;
  "aria-label"?: string;
}

/** Segmented control (active segment on `bg-tile`). */
export function SegmentedTabs<T extends string>({ value, onValueChange, options, className, ...aria }: SegmentedTabsProps<T>) {
  return (
    <div role="tablist" {...aria} className={cn("inline-flex items-center gap-0.5 rounded-[9px] bg-bg p-0.5 border border-border", className)}>
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <button
            key={option.value}
            role="tab"
            type="button"
            aria-selected={selected}
            onClick={() => onValueChange(option.value)}
            className={cn(
              "h-7 rounded-[7px] px-3 text-[12.5px] font-medium outline-none transition-colors",
              selected ? "bg-bg-tile text-fg" : "text-fg-muted hover:text-fg",
            )}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
