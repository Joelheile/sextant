import { useCallback, useEffect, useState } from "react";
import { api, type Permission, type PermissionName } from "./api";
import { t, type Key } from "./i18n";

const PERMISSION_LABEL: Record<PermissionName, Key> = {
  calendar: "permCalendar",
  reminders: "permReminders",
  safari: "permSafari",
  "screen-recording": "permScreen",
  accessibility: "permAccessibility",
  automation: "permAutomation",
};

const LIVE_PANE = {
  "screen-recording": "screen-recording",
  accessibility: "accessibility",
  automation: "automation",
} as const;

/** The permissions the chosen sources need, with a button to allow each one. */
export default function Permissions() {
  const [permissions, setPermissions] = useState<Permission[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setPermissions(await api.permissionStatus());
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const act = async (permission: Permission) => {
    try {
      if (permission.name === "safari") {
        await api.openSettingsPane("full-disk-access");
      } else if (permission.name in LIVE_PANE) {
        const pane = LIVE_PANE[permission.name as keyof typeof LIVE_PANE];
        // macOS shows its prompt once. After that the user has to switch it on in Settings.
        if (permission.status === "denied" || !(await api.requestPermission(permission.name))) {
          await api.openSettingsPane(pane);
        }
        await refresh();
      } else if (permission.status === "undetermined") {
        await api.requestPermission(permission.name);
        await refresh();
      } else {
        await api.openSettingsPane(permission.name === "calendar" ? "calendars" : "reminders");
      }
    } catch (err) {
      setError(String(err));
    }
  };

  const label = (permission: Permission): Key =>
    permission.status === "undetermined" && permission.name !== "safari"
      ? "allow"
      : "openSettings";

  return (
    <>
      {permissions?.length === 0 && <p className="muted">{t("accessNone")}</p>}
      <ul className="choices">
        {permissions?.map((permission) => (
          <li key={permission.name} className="permission">
            <span>
              <strong>{t(PERMISSION_LABEL[permission.name])}</strong>
              <span className={permission.status === "granted" ? "ok" : "muted"}>
                {t(permission.status)}
              </span>
            </span>
            {permission.status !== "granted" && (
              <button type="button" className="secondary" onClick={() => void act(permission)}>
                {t(label(permission))}
              </button>
            )}
          </li>
        ))}
      </ul>
      {permissions?.some((p) => p.name === "safari" && p.status !== "granted") && (
        <p className="muted">{t("safariRestart")}</p>
      )}
      {permissions?.some((p) => p.name === "screen-recording" && p.status !== "granted") && (
        <p className="muted">{t("screenRestart")}</p>
      )}
      {permissions?.some((p) => p.name === "automation" && p.status !== "granted") && (
        <p className="muted">{t("automationHint")}</p>
      )}
      {error && <p role="alert">{error}</p>}
      <button type="button" className="secondary" onClick={() => void refresh()}>
        {t("checkAgain")}
      </button>
    </>
  );
}
