import { useCallback, useEffect, useState } from "react";
import { api, type Permission, type PermissionName, type SettingsPane } from "./api";
import { Check } from "lucide-react";
import { t, type Key } from "./i18n";
import { Failed, List, Muted } from "./layout";
import { Button } from "@/components/ui/button";

export const PERMISSION_LABEL: Record<PermissionName, Key> = {
  "screen-recording": "permScreen",
  accessibility: "permAccessibility",
  automation: "permAutomation",
};

/** Permissions the user grants by dragging Sextant into a System Settings list. */
const DRAG_PANE: Partial<Record<PermissionName, SettingsPane>> = {
  "screen-recording": "screen-recording",
  accessibility: "accessibility",
};

/** These apply only after Sextant starts again. */
const NEEDS_REOPEN: PermissionName[] = ["screen-recording"];

const POLL_MS = 1000;

/** The permissions live capture needs, polled every second, with the actions that grant them. */
export function usePermissions() {
  const [permissions, setPermissions] = useState<Permission[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** The permission "Allow all" is waiting on, or null when it is not running. */
  const [walking, setWalking] = useState<PermissionName | null>(null);
  const [opened, setOpened] = useState<PermissionName[]>([]);

  const refresh = useCallback(async () => {
    try {
      setPermissions(await api.permissionStatus());
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), POLL_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  const act = useCallback(
    async (permission: Permission) => {
      const dragPane = DRAG_PANE[permission.name];
      if (dragPane) {
        await api.openSettingsPane(dragPane, t("dragHint"));
        setOpened((names) => [...names, permission.name]);
      } else if (permission.name === "automation") {
        // macOS shows its prompt once. After that the user has to switch it on in Settings.
        if (permission.status === "denied" || !(await api.requestPermission("automation"))) {
          await api.openSettingsPane("automation");
        }
      }
      await refresh();
    },
    [refresh],
  );

  /** Moves "Allow all" to the first missing permission after `from`. Prompts move on once
   * answered; drag panes wait until they are granted or the user presses Next. */
  const walkFrom = useCallback(
    async (from: PermissionName | null) => {
      try {
        const list = await api.permissionStatus();
        setPermissions(list);
        const start = from === null ? 0 : list.findIndex((p) => p.name === from) + 1;
        const next = list.slice(start).find((p) => p.status !== "granted");
        setWalking(next?.name ?? null);
        if (!next) return;
        await act(next);
        if (!DRAG_PANE[next.name]) await walkFrom(next.name);
      } catch (err) {
        setWalking(null);
        setError(String(err));
      }
    },
    [act],
  );

  useEffect(() => {
    if (!walking || !DRAG_PANE[walking]) return;
    if (permissions?.find((p) => p.name === walking)?.status === "granted") void walkFrom(walking);
  }, [permissions, walking, walkFrom]);

  const allow = (permission: Permission) => act(permission).catch((err) => setError(String(err)));

  const missing = permissions?.filter((p) => p.status !== "granted") ?? [];
  const reopen = missing.some((p) => NEEDS_REOPEN.includes(p.name) && opened.includes(p.name));

  return { permissions, error, walking, walkFrom, allow, missing, reopen };
}

export type PermissionState = ReturnType<typeof usePermissions>;

/** A check when every named permission is granted, else Allow for the first missing one. Shows
 * nothing for permissions live capture does not need. */
export function AllowButton({ state, names }: { state: PermissionState; names: PermissionName[] }) {
  const needed = state.permissions?.filter((p) => names.includes(p.name)) ?? [];
  if (needed.length === 0) return null;
  const missing = needed.find((p) => p.status !== "granted");
  if (!missing) return <Check aria-label={t("granted")} className="size-4 text-success" />;
  return (
    <Button type="button" variant="outline" size="sm" onClick={() => void state.allow(missing)}>
      {t("allow")}
    </Button>
  );
}

/** The reopen prompt, the Automation hint and any error, under a permission list. */
export function PermissionNotes({ state }: { state: PermissionState }) {
  const automationMissing = state.missing.some((p) => p.name === "automation");
  return (
    <>
      {state.reopen && (
        <div className="flex flex-wrap items-center gap-3">
          <Muted>{t("reopenHint")}</Muted>
          <Button type="button" variant="outline" onClick={() => void api.relaunch()}>
            {t("reopen")}
          </Button>
        </div>
      )}
      {automationMissing && <Muted>{t("automationHint")}</Muted>}
      {state.error && <Failed>{state.error}</Failed>}
    </>
  );
}

const label = (permission: Permission): Key =>
  DRAG_PANE[permission.name] || permission.status === "undetermined" ? "allow" : "openSettings";

/** The permissions live capture needs, with a button to allow each one or all in turn. */
export default function Permissions() {
  const state = usePermissions();
  const { permissions, walking, walkFrom, allow, missing } = state;

  return (
    <div className="flex flex-col gap-3">
      {permissions?.length === 0 && <Muted>{t("accessNone")}</Muted>}
      {missing.length > 1 && walking === null && (
        <Button type="button" className="self-start" onClick={() => void walkFrom(null)}>
          {t("allowAll")}
        </Button>
      )}
      {permissions && permissions.length > 0 && (
        <List>
          {permissions.map((permission) => (
            <li
              key={permission.name}
              className="flex items-center justify-between gap-4 px-4 py-3 aria-[current=step]:bg-primary/5"
              aria-current={walking === permission.name ? "step" : undefined}
            >
              <div className="flex flex-col gap-0.5">
                <span className="text-sm font-medium">{t(PERMISSION_LABEL[permission.name])}</span>
                {permission.status === "granted" ? (
                  <span className="flex items-center gap-1 text-xs text-success">
                    <Check aria-hidden className="size-3.5" />
                    {t(permission.status)}
                  </span>
                ) : (
                  <span className="text-xs text-muted-foreground">{t(permission.status)}</span>
                )}
              </div>
              {walking === permission.name ? (
                <Button type="button" onClick={() => void walkFrom(permission.name)}>
                  {t("next")}
                </Button>
              ) : (
                permission.status !== "granted" && (
                  <Button type="button" variant="outline" onClick={() => void allow(permission)}>
                    {t(label(permission))}
                  </Button>
                )
              )}
            </li>
          ))}
        </List>
      )}
      <PermissionNotes state={state} />
    </div>
  );
}
