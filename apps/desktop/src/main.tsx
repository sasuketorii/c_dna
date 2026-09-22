import React, { lazy, Suspense } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import "./styles.css";
const Studio = lazy(() => import("./App").then(({ App }) => ({ default: App })));
const Playground = lazy(() => import("./playground/PublicPlayground").then(({ PublicPlayground }) => ({ default: PublicPlayground })));
const publicMode = import.meta.env.MODE === "static" && new URLSearchParams(location.search).get("studio") !== "1";
const client = new QueryClient({
  defaultOptions: {
    queries: { retry: false, refetchOnWindowFocus: false, staleTime: 15000 },
    mutations: { retry: false },
  },
});
createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={client}>
      <Suspense fallback={<main style={{padding:32,fontFamily:"sans-serif"}}>CEO-DNAを準備しています…</main>}>
        {publicMode ? <Playground /> : <Studio />}
      </Suspense>
    </QueryClientProvider>
  </React.StrictMode>,
);
