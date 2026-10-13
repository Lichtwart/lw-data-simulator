import { KeyRoundIcon, XIcon } from "lucide-react";
import { type FormEvent, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Navigate } from "react-router";

import logoBlue from "@/assets/logos/lichtwart-blue.svg";
import logoWhite from "@/assets/logos/lichtwart-white.svg";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api, errorKey, errorValues, isTauri, type SavedLogin } from "@/lib/api";

import { useAuth } from "./AuthProvider";
import { tenantLabel } from "./tenant";

/**
 * Login (PRD §5): tenant id + URL, user name + password, remember-me. No tenant ships with the
 * app; the remembered logins are the tenant choice.
 */
export function LoginScreen() {
  const { t } = useTranslation();
  const { auth, login, loginSaved } = useAuth();
  const [tenantId, setTenantId] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [remember, setRemember] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState<SavedLogin[]>([]);

  useEffect(() => {
    if (!isTauri()) return;
    api.savedLogins().then(setSaved, () => setSaved([]));
  }, []);

  if (auth) return <Navigate to="/devices" replace />;

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await login({ tenantId, baseUrl, username, password, remember });
    } catch (err) {
      setError(err ?? new Error("unknown"));
    } finally {
      setBusy(false);
    }
  }

  async function onSaved(key: string) {
    setBusy(true);
    setError(null);
    try {
      await loginSaved(key);
    } catch (err) {
      setError(err ?? new Error("unknown"));
    } finally {
      setBusy(false);
    }
  }

  async function onForget(key: string) {
    try {
      await api.forgetLogin(key);
      setSaved((list) => list.filter((l) => l.tenant.key !== key));
    } catch (err) {
      setError(err ?? new Error("unknown"));
    }
  }

  return (
    <div className="flex h-full items-center justify-center p-4 sm:p-8">
      <Card className="w-full max-w-sm">
        <CardHeader>
          <img
            src={logoBlue}
            alt={t("app.brand")}
            className="mb-4 h-7 w-auto self-start dark:hidden"
          />
          <img
            src={logoWhite}
            alt={t("app.brand")}
            className="mb-4 hidden h-7 w-auto self-start dark:block"
          />
          <h1 className="font-display text-[length:var(--lw-fs-h2)] font-light leading-[var(--lw-lh-heading)] text-strong">
            {t("login.title")}
          </h1>
          <CardDescription>{t("login.subtitle")}</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-6">
          {saved.length > 0 && (
            <section className="flex flex-col gap-2" aria-label={t("login.saved")}>
              <h2 className="text-xs font-bold tracking-[0.14em] text-muted-foreground uppercase">
                {t("login.saved")}
              </h2>
              <ul className="flex flex-col gap-2">
                {saved.map((l) => (
                  <li key={l.tenant.key} className="flex items-center gap-2">
                    <Button
                      type="button"
                      variant="outline"
                      className="h-auto flex-1 justify-start py-2 text-left"
                      disabled={busy}
                      onClick={() => void onSaved(l.tenant.key)}
                    >
                      <KeyRoundIcon />
                      <span className="flex flex-col">
                        <span>{l.username}</span>
                        <span className="text-xs font-normal text-muted-foreground">
                          {tenantLabel(l.tenant)}
                        </span>
                      </span>
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      aria-label={t("login.forget", { user: l.username })}
                      onClick={() => void onForget(l.tenant.key)}
                    >
                      <XIcon />
                    </Button>
                  </li>
                ))}
              </ul>
            </section>
          )}
          <form className="flex flex-col gap-4" onSubmit={onSubmit}>
            <div className="flex flex-col gap-2">
              <Label htmlFor="base-url">{t("login.url")}</Label>
              <Input
                id="base-url"
                inputMode="url"
                autoComplete="url"
                spellCheck={false}
                placeholder="https://"
                className="font-mono"
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
              />
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor="tenant-id">{t("login.tenantId")}</Label>
              <Input
                id="tenant-id"
                autoComplete="off"
                spellCheck={false}
                className="font-mono"
                value={tenantId}
                onChange={(e) => setTenantId(e.target.value)}
              />
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor="username">{t("login.username")}</Label>
              <Input
                id="username"
                autoComplete="username"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
              />
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor="password">{t("login.password")}</Label>
              <Input
                id="password"
                type="password"
                autoComplete="current-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </div>
            <Label className="font-normal">
              <Checkbox checked={remember} onCheckedChange={(v) => setRemember(v === true)} />
              {t("login.remember")}
            </Label>
            {error !== null && (
              <p className="text-sm text-destructive">{t(errorKey(error), errorValues(error))}</p>
            )}
            <Button
              type="submit"
              variant="signal"
              disabled={busy || !tenantId.trim() || !baseUrl.trim() || !username || !password}
            >
              {t("login.submit")}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
