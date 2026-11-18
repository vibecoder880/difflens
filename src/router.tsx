//! Code-based TanStack Router tree (SPA mode via RouterProvider).

import { createRootRoute, createRoute, createRouter, Outlet } from "@tanstack/react-router";

import { AppShell } from "@/components/layout/AppShell";
import { DownloadsPage } from "@/routes/Downloads";
import { DiffPage } from "@/routes/Diff";
import { HomePage } from "@/routes/Home";
import { SettingsPage } from "@/routes/Settings";

const rootRoute = createRootRoute({
  component: () => <AppShell />,
  notFoundComponent: () => (
    <div className="flex h-full items-center justify-center text-sm text-muted">
      404 — page not found
    </div>
  ),
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: HomePage,
});

const diffRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/diff/$jobId",
  component: DiffPage,
});

const downloadsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/downloads",
  component: DownloadsPage,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: SettingsPage,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  diffRoute,
  downloadsRoute,
  settingsRoute,
]);

export const router = createRouter({
  routeTree,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

// Re-exported so the shell's <Outlet /> has a typed anchor.
export type RootOutlet = typeof Outlet;
