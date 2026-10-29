import { useId, type ReactNode } from "react";
import { cn } from "@/lib/utils";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";

export function Page({ children }: { children: ReactNode }) {
  return <main className="mx-auto flex w-full max-w-2xl flex-col gap-8 px-6 pt-6 pb-12">{children}</main>;
}

/** One titled block of a screen. */
export function Section({
  title,
  description,
  children,
  className,
}: {
  title?: string;
  description?: string;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("flex flex-col gap-3", className)}>
      {(title || description) && (
        <header className="flex flex-col gap-1">
          {title && <h2 className="text-[15px] font-semibold">{title}</h2>}
          {description && <p className="text-sm text-muted-foreground">{description}</p>}
        </header>
      )}
      {children}
    </section>
  );
}

/** Rows in one bordered box, split by hairlines. */
export function List({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <ul className={cn("divide-y overflow-hidden rounded-xl bg-card ring-1 ring-foreground/10", className)}>
      {children}
    </ul>
  );
}

export function Muted({ children, className }: { children: ReactNode; className?: string }) {
  return <p className={cn("text-sm text-muted-foreground", className)}>{children}</p>;
}

export function Failed({ children }: { children: ReactNode }) {
  return (
    <p role="alert" className="text-sm text-destructive">
      {children}
    </p>
  );
}

export function Done({ children }: { children: ReactNode }) {
  return (
    <p aria-live="polite" className="text-sm text-success">
      {children}
    </p>
  );
}

/** A `List` row: a label on the left, a switch on the right. */
export function SwitchRow({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (on: boolean) => void;
}) {
  const id = useId();
  return (
    <li className="flex items-center justify-between gap-4 px-4 py-3">
      <Label htmlFor={id} className="font-normal leading-snug">
        {label}
      </Label>
      <Switch id={id} checked={checked} onCheckedChange={onChange} />
    </li>
  );
}
