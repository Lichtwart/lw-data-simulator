import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";

import { type AuthState, api, events, isTauri, type LoginRequest } from "@/lib/api";
import { useAppEvent } from "@/lib/useAppEvent";

interface AuthContextValue {
  /** Logged-in user, `null` when logged out. */
  auth: AuthState | null;
  /** True until the initial `currentAuth` check finished. */
  loading: boolean;
  login: (request: LoginRequest) => Promise<AuthState>;
  loginSaved: (tenantKey: string) => Promise<AuthState>;
  logout: () => Promise<void>;
}

const AuthContext = createContext<AuthContextValue | null>(null);

/** Holds the auth state for the UI. Credentials themselves never leave Rust. */
export function AuthProvider({ children }: { children: ReactNode }) {
  const [auth, setAuth] = useState<AuthState | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!isTauri()) {
      setLoading(false);
      return;
    }
    api
      .currentAuth()
      .then(setAuth)
      .catch(() => setAuth(null))
      .finally(() => setLoading(false));
  }, []);

  useAppEvent(events.authExpired, () => setAuth(null));

  const login = useCallback(async (request: LoginRequest) => {
    const user = await api.login(request);
    setAuth(user);
    return user;
  }, []);

  const loginSaved = useCallback(async (tenantKey: string) => {
    const user = await api.loginSaved(tenantKey);
    setAuth(user);
    return user;
  }, []);

  const logout = useCallback(async () => {
    try {
      await api.logout();
    } finally {
      // Even if the backend call fails the user asked to leave: drop the local login.
      setAuth(null);
    }
  }, []);

  const value = useMemo(
    () => ({ auth, loading, login, loginSaved, logout }),
    [auth, loading, login, loginSaved, logout],
  );
  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error("useAuth outside AuthProvider");
  return ctx;
}
