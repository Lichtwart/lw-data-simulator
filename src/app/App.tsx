import { RouterProvider } from "react-router/dom";

import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { AuthProvider } from "@/features/auth/AuthProvider";
import { GlobalEvents } from "@/features/session/GlobalEvents";
import { SessionProvider } from "@/features/session/SessionProvider";

import { router } from "./router";

/** Root component: global providers + router. */
export function App() {
  return (
    <AuthProvider>
      <SessionProvider>
        <TooltipProvider delayDuration={200}>
          <RouterProvider router={router} />
          <GlobalEvents />
          <Toaster position="bottom-left" />
        </TooltipProvider>
      </SessionProvider>
    </AuthProvider>
  );
}
