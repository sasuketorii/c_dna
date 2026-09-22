import { useEffect, useRef, useState } from "react";
import { ExposureHistory } from "./exposure";
import { PolicyPanel } from "./PolicyPanel";
import { CloneSetup } from "./onboarding/CloneSetup";
import { AssessmentPanel } from "./assessment/AssessmentPanel";
import type { ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  SidebarTrigger,
} from "./components/ui/sidebar";
import { TooltipProvider } from "./components/ui/tooltip";
import {
  Empty as EmptyPrimitive,
  EmptyHeader,
  EmptyTitle,
  EmptyDescription,
  EmptyContent,
} from "./components/ui/empty";
import { Button } from "./components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "./components/ui/card";
import { Badge } from "./components/ui/badge";
import { Input } from "./components/ui/input";
import { Textarea } from "./components/ui/textarea";
import {
  command,
  hasSession,
  clearSession,
  rankRequest,
  staticMode,
} from "./api";
import type { Memory, Model, Proposal, Ranking, Response } from "./api";
import { ja, nav, domains, attributes, facts, examples, reasons } from "./ja";
import type { Page } from "./ja";

type Status = {
  workspaces: string[];
  workspace_details?: [string, string][];
  demo: boolean;
  locked: boolean;
};
const initialValues = [
  [0.4, 0.2, 0.9, 0.3, 0.7, 0.3],
  [0.3, 0.8, 0.5, 0.9, 0.8, 0.2],
  [0.8, 0.4, 0.7, 0.5, 0.7, 0.5],
  [0.5, 0.9, 0.2, 1, 0.5, 0.1],
];
function draft(workspace: string, index: number): Proposal {
  const e = examples[index % examples.length];
  const now = new Date().toISOString();
  return {
    schema_version: "1.0",
    request_id: crypto.randomUUID(),
    workspace_id: workspace,
    family_id: crypto.randomUUID(),
    case_kind: "hypothetical",
    domain: e.domain,
    context: {
      as_of: now,
      summary: e.summary,
      facts: Object.keys(facts).map((key, i) => ({
        key,
        value: [0.8, 0.7, 0.3, 0.7, 0.6][i],
        evidence_status: "explicit",
        unit: "ratio",
      })),
      unknown_fields: [],
    },
    candidates: e.candidates.map((text, i) => ({
      id: `option_${i + 1}`,
      text,
      attributes: Object.fromEntries(
        Object.keys(attributes).map((key, j) => [key, initialValues[i][j]]),
      ),
    })),
    response: { response_type: "skip" },
    source: {
      artifact_id: crypto.randomUUID(),
      source_kind: "human_app",
      occurred_at: null,
      observed_at: now,
    },
    rationale_explicit: null,
    reversal_conditions: [],
    model_exposure: false,
  };
}
function Panel({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children?: ReactNode;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        {description && <CardDescription>{description}</CardDescription>}
      </CardHeader>
      {children && <CardContent>{children}</CardContent>}
    </Card>
  );
}
function Empty({
  title,
  body,
  children,
}: {
  title: string;
  body: string;
  children?: ReactNode;
}) {
  return (
    <EmptyPrimitive className="border">
      <EmptyHeader>
        <EmptyTitle>{title}</EmptyTitle>
        <EmptyDescription>{body}</EmptyDescription>
      </EmptyHeader>
      {children && <EmptyContent>{children}</EmptyContent>}
    </EmptyPrimitive>
  );
}
function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-2 text-sm font-medium">
      {label}
      {children}
    </label>
  );
}
function responseLabel(p: Proposal) {
  const r = p.response;
  if (r.response_type === "acceptability")
    return `${r.outcome === "accept" ? ja.accept : r.outcome === "reject" ? ja.reject : r.outcome === "conditional" ? ja.tie : ja.insufficient} · ${p.candidates.find((c) => c.id === r.candidate_id)?.text || ""}`;
  if (r.candidate_id)
    return (
      p.candidates.find((c) => c.id === r.candidate_id)?.text || r.candidate_id
    );
  return (
    (
      {
        skip: ja.skip,
        none_fit: ja.noneFit,
        need_information: ja.insufficient,
        choose_set: ja.tie,
        pairwise:
          r.outcome === "left"
            ? ja.left
            : r.outcome === "right"
              ? ja.right
              : ja.tie,
      } as Record<string, string>
    )[r.response_type] || r.response_type
  );
}

export function App() {
  const client = useQueryClient();
  const [workspace, setWorkspace] = useState("");
  const [page, setPage] = useState<Page>("home");
  const [theme, setTheme] = useState(false);
  const [zone, setZone] = useState("local");
  const [locked, setLocked] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState("");
  const busyRef = useRef(false);
  const exposure = useRef(new ExposureHistory());
  const sourceRecordId = useRef<string | null>(null);
  const [workspaceName, setWorkspaceName] = useState("");
  const [names, setNames] = useState<Record<string, string>>({});
  const [offset, setOffset] = useState(0);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const [editing, setEditing] = useState<Memory | null>(null);
  const [deleting, setDeleting] = useState<Memory | null>(null);
  const [index, setIndex] = useState(0);
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [selected, setSelected] = useState(false);
  const [saved, setSaved] = useState<Memory | null>(null);
  const [kind, setKind] = useState("choose_one");
  const [immediate, setImmediate] = useState(false);
  const [sessionCount, setSessionCount] = useState(0);
  const [sessionSize, setSessionSize] = useState("1");
  const [rank, setRank] = useState<Ranking | null>(null);
  const [preview, setPreview] = useState(false);
  const [modelId, setModelId] = useState("");
  const [editOpen, setEditOpen] = useState(false);
  const noteRef = useRef<HTMLTextAreaElement>(null);
  const answerRef = useRef<HTMLDivElement>(null);
  const pointer = useRef<number | null>(null);
  const status = useQuery({
    queryKey: ["status"],
    queryFn: () => command<Status>("status"),
    enabled: hasSession() && !locked,
  });
  const active = workspace || status.data?.workspaces[0] || "";
  function workspaceCreated(created: { id: string; name: string }) {
    setNames((names) => ({ ...names, [created.id]: created.name }));
    setWorkspace(created.id);
    setPage("home");
    void client.invalidateQueries({ queryKey: ["status"] });
  }
  const records = useQuery({
    queryKey: ["records", active, offset],
    queryFn: () =>
      command<{ records: Memory[] }>("list", { workspace_id: active, offset }),
    enabled: !!active && !locked,
  });
  const models = useQuery({
    queryKey: ["models", active],
    queryFn: () =>
      command<{ models: Model[] }>("models", { workspace_id: active }),
    enabled: !!active && !locked,
  });
  const gaps = useQuery({
    queryKey: ["gaps", active],
    queryFn: () =>
      command<{
        domains: {
          domain: string;
          confirmed_records: number;
          status: string;
        }[];
        counts_truncated: boolean;
      }>("gaps", { workspace_id: active }),
    enabled: !!active && !locked,
  });
  useEffect(() => {
    const listener = (event: Event) =>
      setProgress(String((event as CustomEvent).detail));
    window.addEventListener("cdna-runtime-progress", listener);
    return () => window.removeEventListener("cdna-runtime-progress", listener);
  }, []);
  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme);
  }, [theme]);
  useEffect(() => {
    if (active) {
      sourceRecordId.current = null;
      setProposal(draft(active, 0));
      setIndex(0);
      setSaved(null);
      setSelected(false);
      setRank(null);
      setEditing(null);
      setDeleting(null);
      setOffset(0);
      setSessionCount(0);
      setModelId("");
    }
  }, [active]);
  const memories = records.data?.records || [];
  const confirmed =
    gaps.data?.domains.reduce((n, d) => n + d.confirmed_records, 0) || 0;
  function date(value: string) {
    return new Intl.DateTimeFormat("ja-JP", {
      dateStyle: "medium",
      timeStyle: "short",
      ...(zone === "utc" ? { timeZone: "UTC" } : {}),
    }).format(new Date(value));
  }
  async function act(fn: () => Promise<void>) {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setProgress("");
    setError("");
    setNotice("");
    try {
      await fn();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }
  async function undoSaved() {
    if (!saved || !proposal) return;
    await command("delete", {
      workspace_id: active,
      id: saved.id,
      expected_revision: saved.revision,
    });
    setSaved(null);
    setProposal({ ...proposal, request_id: crypto.randomUUID() });
    setSessionCount((c) => Math.max(0, c - 1));
    await refresh();
  }
  async function refresh() {
    await client.invalidateQueries({ queryKey: ["records", active] });
    await client.invalidateQueries({ queryKey: ["models", active] });
    await client.invalidateQueries({ queryKey: ["gaps", active] });
  }
  async function save(p: Proposal) {
    p = {
      ...exposure.current.apply(p, editing?.id ?? sourceRecordId.current),
      reversal_conditions: p.reversal_conditions.filter((v) => v.trim()),
    };
    const result = editing
      ? await command<Memory>("revise", {
          workspace_id: active,
          id: editing.id,
          expected_revision: editing.revision,
          request_id: p.request_id,
          proposal: p,
        })
      : await command<Memory>("propose", { proposal: p });
    sourceRecordId.current = result.id;
    setProposal(p);
    setSaved(result);
    setEditing(null);
    setSessionCount((c) => c + 1);
    setNotice(ja.saved);
    await refresh();
  }
  function choose(response: Response) {
    if (busy || saved || !proposal) return;
    const p = { ...proposal, response };
    setProposal(p);
    setSelected(true);
    if (immediate) void act(() => save(p));
  }
  function resetQuestion() {
    const next = index + 1;
    setIndex(next);
    sourceRecordId.current = null;
    setProposal(draft(active, next));
    setSaved(null);
    setSelected(false);
    setEditing(null);
    setRank(null);
    setEditOpen(false);
  }
  function revise(r: Memory) {
    sourceRecordId.current = r.id;
    setProposal({
      ...exposure.current.apply(r.payload, r.id),
      request_id: crypto.randomUUID(),
    });
    setEditing(r);
    setSaved(null);
    setSelected(true);
    setKind(
      r.payload.response.response_type === "pairwise"
        ? "pairwise"
        : r.payload.response.response_type === "acceptability"
          ? "acceptability"
          : "choose_one",
    );
    setPage("grow");
    setEditOpen(true);
    setRank(null);
  }
  function keydown(e: React.KeyboardEvent) {
    const target = e.target as HTMLElement;
    if (
      e.nativeEvent.isComposing ||
      e.repeat ||
      busy ||
      deleting ||
      target.closest(
        'input,textarea,select,[contenteditable="true"],dialog,[role="dialog"]',
      )
    )
      return;
    if (e.key === "n" || e.key === "N") {
      e.preventDefault();
      noteRef.current?.focus();
      return;
    }
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z") {
      e.preventDefault();
      if (saved) void act(undoSaved);
      else setSelected(false);
      return;
    }
    if (saved) return;
    if (kind === "choose_one" && /^[1-4]$/.test(e.key)) {
      e.preventDefault();
      choose({
        response_type: "choose_one",
        candidate_id: proposal?.candidates[Number(e.key) - 1]?.id,
      });
    } else if (
      kind !== "choose_one" &&
      (e.key === "ArrowLeft" || e.key === "ArrowRight")
    ) {
      e.preventDefault();
      compare(e.key === "ArrowLeft" ? "left" : "right");
    } else if (e.key === "Enter" && selected && proposal) {
      e.preventDefault();
      void act(() => save(proposal));
    } else if (e.key === " " && answerRef.current?.contains(target)) {
      e.preventDefault();
      choose({ response_type: "skip" });
    }
  }
  function compare(outcome: string) {
    if (!proposal) return;
    choose(
      kind === "pairwise"
        ? {
            response_type: "pairwise",
            left_id: proposal.candidates[0].id,
            right_id: proposal.candidates[1].id,
            outcome,
          }
        : {
            response_type: "acceptability",
            candidate_id: proposal.candidates[0].id,
            outcome:
              outcome === "left"
                ? "reject"
                : outcome === "right"
                  ? "accept"
                  : outcome === "tie"
                    ? "conditional"
                    : "insufficient",
          },
    );
  }
  async function infer(useModel: boolean) {
    if (!proposal) return;
    const request = rankRequest(proposal);
    if (useModel) {
      exposure.current.mark(proposal, sourceRecordId.current);
      setProposal(exposure.current.apply(proposal, sourceRecordId.current));
      setPreview(true);
      setRank(
        await command<Ranking>("preview_model", {
          workspace_id: active,
          id: modelId || models.data?.models[0]?.id,
          request,
        }),
      );
    } else {
      setPreview(false);
      setRank(await command<Ranking>("rank", { request }));
    }
  }
  const queryError =
    status.error || records.error || models.error || gaps.error;
  const subtitle =
    (
      {
        home: ja.homeDescription,
        grow: ja.trainingDescription,
        judge: ja.judgmentDescription,
        memory: ja.memoryDescription,
        map: ja.mapDescription,
        models: ja.modelsDescription,
      } as Partial<Record<Page, string>>
    )[page] || ja.local;
  if (locked)
    return (
      <main className="mx-auto max-w-xl p-8 pt-24">
        <Empty title={ja.locked} body={ja.lockedBody} />
      </main>
    );
  return (
    <TooltipProvider>
      <SidebarProvider>
        <Sidebar collapsible="offcanvas">
          <SidebarHeader className="gap-5 p-5">
            <a
              href="#"
              onClick={(e) => {
                e.preventDefault();
                setPage("home");
              }}
              className="text-xl font-semibold tracking-tight"
            >
              {ja.brand}
              <span className="mt-1 block text-xs font-normal text-muted-foreground">
                {ja.tagline}
              </span>
            </a>
            <Field label={ja.workspace}>
              <select
                aria-label={ja.workspace}
                className="control"
                value={active}
                onChange={(e) => {
                  setWorkspace(e.target.value);
                  setNotice("");
                  setError("");
                }}
                disabled={busy}
              >
                {!active && <option value="">{ja.newWorkspace}</option>}
                {status.data?.workspaces.map((w) => (
                  <option key={w} value={w}>
                    {names[w] ||
                      status.data?.workspace_details?.find(
                        ([id]) => id === w,
                      )?.[1] ||
                      `${ja.workspace} ${w.slice(0, 8)}`}
                  </option>
                ))}
              </select>
            </Field>
          </SidebarHeader>
          <SidebarContent>
            <SidebarGroup>
              <SidebarGroupContent>
                <SidebarMenu>
                  {Object.entries(nav).map(([id, title]) => (
                    <SidebarMenuItem key={id}>
                      <SidebarMenuButton
                        isActive={page === id}
                        onClick={() => setPage(id as Page)}
                        aria-current={page === id ? "page" : undefined}
                        className="h-10 px-4"
                      >
                        <span>{title}</span>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  ))}
                </SidebarMenu>
              </SidebarGroupContent>
            </SidebarGroup>
          </SidebarContent>
          <SidebarFooter className="p-5 text-xs leading-6 text-muted-foreground">
            <p>
              {ja.local}
              <br />
              {ja.unverified}
              <br />
              {ja.none}
            </p>
            <span>C-DNA · LOCAL FIRST</span>
          </SidebarFooter>
        </Sidebar>
        <SidebarInset onKeyDown={page === "grow" ? keydown : undefined}>
          <header className="flex h-16 shrink-0 items-center gap-2 border-b bg-zinc-950 text-white">
            <div className="flex w-full items-center gap-3 px-4">
              <SidebarTrigger aria-label={ja.navigation} />
              <span className="text-sm font-medium">{ja.brand}</span>
              <span className="ml-auto text-xs">
                {status.data?.demo ? ja.demo : ja.local}
              </span>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setTheme(!theme)}
                aria-label={ja.theme}
              >
                {theme ? "☀" : "◐"}
              </Button>
            </div>
          </header>
          <div className="px-6 pt-6">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div>
                <h1 className="text-xl font-semibold tracking-tight">
                  {nav[page]}
                </h1>
                <p className="pt-1 text-sm text-muted-foreground">{subtitle}</p>
              </div>
              <Badge variant="outline">
                {status.isPending
                  ? ja.loading
                  : status.data
                    ? ja.ready
                    : ja.noConnection}
              </Badge>
            </div>
          </div>
          <div className="mx-auto flex max-w-6xl flex-col gap-6 p-5 sm:p-8">
            {staticMode && (
              <p
                role="note"
                className="rounded-lg border bg-muted/50 p-3 text-xs leading-6 text-muted-foreground"
              >
                {ja.privacy}
              </p>
            )}
            {(error || queryError) && (
              <div
                role="alert"
                className="rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm"
              >
                <p>{error || (queryError as Error)?.message}</p>
                <Button
                  className="mt-3"
                  variant="outline"
                  onClick={() =>
                    void act(async () => {
                      await client.invalidateQueries();
                    })
                  }
                  disabled={busy}
                >
                  {ja.refresh}
                </Button>
              </div>
            )}
            {busy && progress && (
              <p role="status" className="text-sm text-muted-foreground">
                {progress}
              </p>
            )}
            {notice && (
              <p
                role="status"
                className="rounded-lg border border-primary/20 bg-primary/5 p-3 text-sm"
              >
                {notice}
              </p>
            )}
            {!hasSession() ? (
              <Empty title={ja.noConnection} body={ja.reconnect} />
            ) : status.isPending ? (
              <Empty title={ja.loading} body={ja.local} />
            ) : !active ? (
              <CloneSetup onWorkspaceCreated={workspaceCreated} />
            ) : (
              <>
                {page === "home" && (
                  <>
                    <CloneSetup
                      workspaceId={active}
                      onImported={() => {
                        void client.invalidateQueries({ queryKey: ["sources", active] });
                      }}
                    />
                    <Panel title={ja.welcome} description={ja.welcomeBody}>
                      <div className="flex gap-3">
                        <Button
                          onClick={() => {
                            if (saved) resetQuestion();
                            setPage("grow");
                          }}
                        >
                          {ja.seed} <span aria-hidden="true">→</span>
                        </Button>
                        <Button
                          variant="outline"
                          onClick={() => setPage("memory")}
                        >
                          {nav.memory}
                        </Button>
                      </div>
                    </Panel>
                    <div className="grid gap-4 sm:grid-cols-3">
                      <Panel
                        title={ja.firstStep}
                        description={ja.trainingDescription}
                      >
                        <Button
                          variant="outline"
                          onClick={() => {
                            if (saved) resetQuestion();
                            setPage("grow");
                          }}
                        >
                          {nav.grow}
                        </Button>
                      </Panel>
                      <Panel
                        title={ja.secondStep}
                        description={ja.proposalNotice}
                      >
                        <p className="text-3xl font-semibold">
                          {confirmed}
                          <span className="ml-2 text-sm font-normal text-muted-foreground">
                            {ja.count} · {ja.confirmed}
                          </span>
                        </p>
                        <Button
                          variant="outline"
                          onClick={() => setPage("memory")}
                        >
                          {nav.memory}
                        </Button>
                      </Panel>
                      <Panel title={ja.thirdStep} description={ja.noAccuracy}>
                        <p className="text-3xl font-semibold">
                          {models.data?.models.length || 0}
                          <span className="ml-2 text-sm font-normal text-muted-foreground">
                            {ja.count} · {ja.provisional}
                          </span>
                        </p>
                        <Button
                          variant="outline"
                          onClick={() => setPage("models")}
                        >
                          {nav.models}
                        </Button>
                      </Panel>
                    </div>
                    <Panel
                      title={ja.nowQuestion}
                      description={
                        domains[proposal?.domain || "resource_allocation"]
                      }
                    >
                      <h2 className="text-lg font-medium">
                        {examples[index % examples.length].title}
                      </h2>
                      <p className="text-sm leading-7 text-muted-foreground">
                        {examples[index % examples.length].purpose}
                      </p>
                      <Button
                        variant="outline"
                        className="self-start"
                        onClick={() => {
                          if (saved) resetQuestion();
                          setPage("grow");
                        }}
                      >
                        {ja.start}
                      </Button>
                    </Panel>
                  </>
                )}
                {(page === "grow" || page === "judge") && proposal && (
                  <>
                    {page === "grow" && (
                      <div className="flex flex-wrap items-center justify-between gap-3">
                        <div className="flex gap-2">
                          {["choose_one", "pairwise", "acceptability"].map(
                            (k, i) => (
                              <Button
                                key={k}
                                variant={kind === k ? "secondary" : "ghost"}
                                size="sm"
                                disabled={busy || !!saved}
                                onClick={() => {
                                  setKind(k);
                                  setSelected(false);
                                }}
                              >
                                {[ja.choice, ja.pairwise, ja.acceptability][i]}
                              </Button>
                            ),
                          )}
                        </div>
                        <div className="flex items-center gap-2 text-xs text-muted-foreground">
                          <label htmlFor="session-size">{ja.session}</label>
                          <select
                            id="session-size"
                            value={sessionSize}
                            onChange={(e) => setSessionSize(e.target.value)}
                            className="control compact"
                          >
                            <option value="1">{ja.one}</option>
                            <option value="3">{ja.three}</option>
                            <option value="0">{ja.free}</option>
                          </select>
                          <span>
                            {ja.completed}: {sessionCount}
                          </span>
                        </div>
                      </div>
                    )}
                    <Panel
                      title={
                        editing
                          ? ja.revise
                          : examples[index % examples.length].title
                      }
                    >
                      <div className="flex flex-wrap gap-2">
                        <Badge variant="secondary">
                          {proposal.case_kind === "hypothetical"
                            ? ja.hypothetical
                            : ja.actual}
                        </Badge>
                        <Badge variant="outline">
                          {domains[proposal.domain]}
                        </Badge>
                        <Badge variant="outline">
                          {saved ? ja.pending : ja.draft}
                        </Badge>
                        {proposal.model_exposure && (
                          <Badge>{ja.modelExposure}</Badge>
                        )}
                      </div>
                      <dl className="grid gap-3 border-b pb-4 text-sm sm:grid-cols-2">
                        <div>
                          <dt className="text-xs text-muted-foreground">
                            {ja.purpose}
                          </dt>
                          <dd className="pt-1 leading-6">
                            {examples[index % examples.length].purpose}
                          </dd>
                        </div>
                        <div>
                          <dt className="text-xs text-muted-foreground">
                            {ja.constraints}
                          </dt>
                          <dd className="pt-1 leading-6">
                            {examples[index % examples.length].constraints}
                          </dd>
                        </div>
                      </dl>
                      <h3 className="text-sm font-medium">{ja.background}</h3>
                      <p className="whitespace-pre-wrap text-sm leading-7">
                        {proposal.context.summary}
                      </p>
                      <p className="text-xs text-muted-foreground">
                        {ja.sourceNote}
                      </p>
                      <details
                        open={editOpen}
                        onToggle={(e) => setEditOpen(e.currentTarget.open)}
                      >
                        <summary className="cursor-pointer py-2 text-sm text-primary">
                          {ja.edit}
                        </summary>
                        <fieldset
                          disabled={busy || !!saved}
                          className="mt-3 flex flex-col gap-4"
                        >
                          <Field label={ja.selectDomain}>
                            <select
                              className="control"
                              value={proposal.domain}
                              onChange={(e) =>
                                setProposal({
                                  ...proposal,
                                  domain: e.target.value,
                                })
                              }
                            >
                              {Object.entries(domains).map(([k, v]) => (
                                <option key={k} value={k}>
                                  {v}
                                </option>
                              ))}
                            </select>
                          </Field>
                          <Field label={ja.background}>
                            <Textarea
                              rows={5}
                              maxLength={8000}
                              value={proposal.context.summary}
                              onChange={(e) =>
                                setProposal({
                                  ...proposal,
                                  context: {
                                    ...proposal.context,
                                    summary: e.target.value,
                                  },
                                })
                              }
                            />
                          </Field>
                          <div className="grid gap-3 sm:grid-cols-3">
                            {proposal.context.facts.map((f, i) => (
                              <Field key={f.key} label={facts[f.key] || f.key}>
                                <Input
                                  type="number"
                                  min="0"
                                  max="1"
                                  step="0.1"
                                  value={f.value}
                                  onChange={(e) =>
                                    setProposal({
                                      ...proposal,
                                      context: {
                                        ...proposal.context,
                                        facts: proposal.context.facts.map(
                                          (x, j) =>
                                            i === j
                                              ? {
                                                  ...x,
                                                  value: e.target.valueAsNumber,
                                                }
                                              : x,
                                        ),
                                      },
                                    })
                                  }
                                />
                              </Field>
                            ))}
                          </div>
                          <Field label={ja.unknown}>
                            <Input
                              key={proposal.request_id}
                              defaultValue={proposal.context.unknown_fields.join(
                                ", ",
                              )}
                              onBlur={(e) =>
                                setProposal({
                                  ...proposal,
                                  context: {
                                    ...proposal.context,
                                    unknown_fields: e.target.value
                                      .split(",")
                                      .map((x) => x.trim())
                                      .filter(Boolean),
                                  },
                                })
                              }
                            />
                            <span className="text-xs font-normal text-muted-foreground">
                              {ja.unknownHelp}
                            </span>
                          </Field>
                          {proposal.candidates.map((c, i) => (
                            <div
                              key={c.id}
                              className="flex flex-col gap-3 rounded-lg border p-4"
                            >
                              <Field label={`${ja.candidates} ${i + 1}`}>
                                <Textarea
                                  maxLength={2000}
                                  value={c.text}
                                  onChange={(e) =>
                                    setProposal({
                                      ...proposal,
                                      candidates: proposal.candidates.map(
                                        (x, j) =>
                                          i === j
                                            ? { ...x, text: e.target.value }
                                            : x,
                                      ),
                                    })
                                  }
                                />
                              </Field>
                              <p className="text-xs text-muted-foreground">
                                {ja.attributes}
                              </p>
                              <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
                                {Object.entries(attributes).map(
                                  ([key, label]) => (
                                    <Field key={key} label={label}>
                                      <Input
                                        type="number"
                                        min="0"
                                        max="1"
                                        step="0.1"
                                        value={c.attributes[key] ?? 0}
                                        onChange={(e) =>
                                          setProposal({
                                            ...proposal,
                                            candidates: proposal.candidates.map(
                                              (x, j) =>
                                                i === j
                                                  ? {
                                                      ...x,
                                                      attributes: {
                                                        ...x.attributes,
                                                        [key]:
                                                          e.target
                                                            .valueAsNumber,
                                                      },
                                                    }
                                                  : x,
                                            ),
                                          })
                                        }
                                      />
                                    </Field>
                                  ),
                                )}
                              </div>
                            </div>
                          ))}
                        </fieldset>
                      </details>
                    </Panel>
                    {page === "grow" ? (
                      <div
                        ref={answerRef}
                        tabIndex={0}
                        className="flex flex-col gap-4 rounded-xl outline-offset-4"
                        aria-label={ja.candidates}
                        onPointerDown={(e) => {
                          if (
                            !(e.target as HTMLElement).closest(
                              "button,input,textarea",
                            )
                          )
                            pointer.current = e.clientX;
                        }}
                        onPointerUp={(e) => {
                          if (
                            pointer.current !== null &&
                            kind !== "choose_one" &&
                            Math.abs(e.clientX - pointer.current) > 80
                          )
                            compare(
                              e.clientX < pointer.current ? "left" : "right",
                            );
                          pointer.current = null;
                        }}
                      >
                        <div
                          className={`grid gap-3 ${kind === "acceptability" ? "" : "md:grid-cols-2"}`}
                        >
                          {proposal.candidates
                            .slice(
                              0,
                              kind === "choose_one"
                                ? 4
                                : kind === "pairwise"
                                  ? 2
                                  : 1,
                            )
                            .map((c, i) => {
                              const isSelected =
                                selected &&
                                (proposal.response.candidate_id === c.id ||
                                  (proposal.response.response_type ===
                                    "pairwise" &&
                                    proposal.response.outcome ===
                                      (i === 0 ? "left" : "right")));
                              return (
                                <Button
                                  key={c.id}
                                  variant="outline"
                                  disabled={busy || !!saved}
                                  aria-pressed={isSelected}
                                  className={`h-auto min-h-28 justify-start whitespace-normal rounded-xl p-5 text-left ${isSelected ? "border-primary bg-primary/5 ring-1 ring-primary" : ""}`}
                                  onClick={() =>
                                    kind === "choose_one"
                                      ? choose({
                                          response_type: "choose_one",
                                          candidate_id: c.id,
                                        })
                                      : kind === "pairwise"
                                        ? compare(i === 0 ? "left" : "right")
                                        : compare("right")
                                  }
                                >
                                  <span className="mr-3 flex size-7 shrink-0 items-center justify-center rounded-md border text-xs">
                                    {isSelected ? "✓" : i + 1}
                                  </span>
                                  <span className="text-sm font-normal leading-7">
                                    {c.text}
                                    {isSelected && (
                                      <span className="mt-1 block text-xs font-medium text-primary">
                                        {ja.selected}
                                      </span>
                                    )}
                                  </span>
                                </Button>
                              );
                            })}
                        </div>
                        {kind === "acceptability" && (
                          <div className="flex gap-2">
                            <Button
                              variant="outline"
                              disabled={busy || !!saved}
                              onClick={() => compare("left")}
                            >
                              {ja.reject}
                            </Button>
                            <Button
                              variant="outline"
                              disabled={busy || !!saved}
                              onClick={() => compare("right")}
                            >
                              {ja.accept}
                            </Button>
                          </div>
                        )}
                        <div className="flex flex-wrap gap-2">
                          {[
                            [ja.insufficient, "need_information"],
                            [ja.noneFit, "none_fit"],
                            [ja.tie, "choose_set"],
                            [ja.skip, "skip"],
                          ].map(([label, type]) => (
                            <Button
                              key={type}
                              variant={
                                selected &&
                                proposal.response.response_type === type
                                  ? "secondary"
                                  : "ghost"
                              }
                              size="sm"
                              disabled={busy || !!saved}
                              onClick={() =>
                                type === "need_information"
                                  ? choose({
                                      response_type: type,
                                      fields: proposal.context.unknown_fields
                                        .length
                                        ? proposal.context.unknown_fields
                                        : ["decision_context"],
                                    })
                                  : type === "choose_set"
                                    ? kind === "choose_one"
                                      ? choose({
                                          response_type: type,
                                          candidate_ids:
                                            proposal.candidates.map(
                                              (c) => c.id,
                                            ),
                                        })
                                      : compare("tie")
                                    : choose({ response_type: type })
                              }
                            >
                              {label}
                            </Button>
                          ))}
                        </div>
                        <Panel title={ja.notes}>
                          <Field label={ja.notes}>
                            <Textarea
                              ref={noteRef}
                              placeholder={ja.notesPlaceholder}
                              disabled={busy || !!saved}
                              maxLength={2000}
                              value={proposal.rationale_explicit || ""}
                              onChange={(e) =>
                                setProposal({
                                  ...proposal,
                                  rationale_explicit: e.target.value || null,
                                })
                              }
                            />
                          </Field>
                          <Field label={ja.reversal}>
                            <Textarea
                              placeholder={ja.reversalPlaceholder}
                              disabled={busy || !!saved}
                              maxLength={500}
                              value={proposal.reversal_conditions.join("\n")}
                              onChange={(e) =>
                                setProposal({
                                  ...proposal,
                                  reversal_conditions:
                                    e.target.value.split("\n"),
                                })
                              }
                            />
                          </Field>
                          <label className="flex items-center gap-2 text-sm">
                            <input
                              type="checkbox"
                              checked={immediate}
                              disabled={busy || !!saved}
                              onChange={(e) => setImmediate(e.target.checked)}
                            />
                            {ja.immediate}
                          </label>
                          <p className="text-xs text-muted-foreground">
                            {ja.immediateHelp}
                          </p>
                        </Panel>
                        <div className="sticky bottom-0 z-10 flex flex-wrap items-center justify-between gap-3 rounded-xl border bg-background/95 p-4 backdrop-blur">
                          <p className="text-xs text-muted-foreground">
                            {kind === "choose_one"
                              ? ja.keyboard
                              : ja.compareKeyboard}
                          </p>
                          <div className="flex gap-2">
                            {saved ? (
                              <>
                                <Button
                                  variant="ghost"
                                  disabled={busy}
                                  onClick={() => void act(undoSaved)}
                                >
                                  {ja.undo}
                                </Button>
                                <Button
                                  variant="outline"
                                  onClick={() => setPage("memory")}
                                >
                                  {nav.memory}
                                </Button>
                                <Button
                                  disabled={busy}
                                  onClick={() =>
                                    Number(sessionSize) > 0 &&
                                    sessionCount >= Number(sessionSize)
                                      ? setPage("home")
                                      : resetQuestion()
                                  }
                                >
                                  {Number(sessionSize) > 0 &&
                                  sessionCount >= Number(sessionSize)
                                    ? ja.finish
                                    : ja.next}
                                </Button>
                              </>
                            ) : (
                              <>
                                <Button
                                  variant="ghost"
                                  disabled={busy || !selected}
                                  onClick={() => setSelected(false)}
                                >
                                  {ja.undo}
                                </Button>
                                <Button
                                  disabled={busy || !selected}
                                  onClick={() => void act(() => save(proposal))}
                                >
                                  {busy
                                    ? ja.wait
                                    : editing
                                      ? ja.revise
                                      : ja.save}
                                </Button>
                              </>
                            )}
                          </div>
                        </div>
                      </div>
                    ) : (
                      <>
                        <Panel title={ja.rank} description={ja.rankHelp}>
                          <Button
                            className="self-start"
                            disabled={busy}
                            onClick={() => void act(() => infer(false))}
                          >
                            {busy ? ja.wait : ja.rank}
                          </Button>
                        </Panel>
                        <Panel title={ja.preview} description={ja.previewHelp}>
                          {models.data?.models.length ? (
                            <>
                              <Field label={ja.provisional}>
                                <select
                                  className="control"
                                  value={modelId || models.data.models[0].id}
                                  onChange={(e) => setModelId(e.target.value)}
                                >
                                  {models.data.models.map((m) => (
                                    <option value={m.id} key={m.id}>
                                      {date(m.created_at)} · {m.id.slice(0, 8)}
                                    </option>
                                  ))}
                                </select>
                              </Field>
                              <Button
                                variant="outline"
                                className="self-start"
                                disabled={busy}
                                onClick={() => void act(() => infer(true))}
                              >
                                {ja.preview}
                              </Button>
                            </>
                          ) : (
                            <p className="text-sm text-muted-foreground">
                              {ja.previewEmpty}
                            </p>
                          )}
                        </Panel>
                        {rank && (
                          <Panel
                            title={
                              preview
                                ? ja.provisional
                                : rank.abstained
                                  ? ja.held
                                  : ja.matched
                            }
                            description={ja.none}
                          >
                            <Badge variant="outline">{ja.unverified}</Badge>
                            {rank.abstention_reasons?.map((r) => (
                              <p key={r} className="text-sm">
                                {reasons[r] || r}
                              </p>
                            ))}
                            {rank.ranking.map((r) => (
                              <div
                                key={r.candidate_id}
                                className="rounded-lg border p-3"
                              >
                                <p className="text-sm leading-6">
                                  {
                                    proposal.candidates.find(
                                      (c) => c.id === r.candidate_id,
                                    )?.text
                                  }
                                </p>
                                {r.memory_match && (
                                  <Badge className="mt-2" variant="secondary">
                                    {ja.matched}
                                  </Badge>
                                )}
                                {r.raw_score !== null && (
                                  <p className="mt-2 text-xs text-muted-foreground">
                                    {ja.score}: {r.raw_score.toFixed(4)}
                                  </p>
                                )}
                              </div>
                            ))}
                            {rank.evidence?.map((e) => (
                              <p
                                className="text-xs text-muted-foreground"
                                key={e.record_id}
                              >
                                {ja.evidence}: {e.record_id} · {ja.revision}{" "}
                                {e.revision}
                              </p>
                            ))}
                          </Panel>
                        )}
                      </>
                    )}
                  </>
                )}
                {page === "memory" && (
                  <>
                    <div className="flex flex-wrap gap-3">
                      <Input
                        className="max-w-sm"
                        aria-label={ja.search}
                        placeholder={ja.search}
                        value={search}
                        onChange={(e) => setSearch(e.target.value)}
                      />
                      <select
                        className="control w-auto"
                        aria-label={ja.status}
                        value={filter}
                        onChange={(e) => setFilter(e.target.value)}
                      >
                        <option value="all">{ja.all}</option>
                        <option value="pending">{ja.pending}</option>
                        <option value="confirmed">{ja.confirmed}</option>
                      </select>
                    </div>
                    {memories.length === 0 ? (
                      <Empty title={ja.noMemory} body={ja.noMemoryBody}>
                        <Button
                          onClick={() => {
                            if (saved) resetQuestion();
                            setPage("grow");
                          }}
                        >
                          {ja.seed}
                        </Button>
                      </Empty>
                    ) : (
                      memories
                        .filter(
                          (r) =>
                            (filter === "all" || r.status === filter) &&
                            `${r.payload.context.summary} ${r.payload.rationale_explicit}`.includes(
                              search,
                            ),
                        )
                        .map((r) => (
                          <Panel
                            key={r.id}
                            title={domains[r.payload.domain]}
                            description={`${date(r.payload.source.observed_at)} · ${ja.revision} ${r.revision}`}
                          >
                            <div className="flex flex-wrap gap-2">
                              <Badge
                                variant={
                                  r.status === "confirmed"
                                    ? "default"
                                    : "secondary"
                                }
                              >
                                {r.status === "confirmed"
                                  ? ja.confirmed
                                  : ja.pending}
                              </Badge>
                              <Badge variant="outline">
                                {r.payload.case_kind === "hypothetical"
                                  ? ja.hypothetical
                                  : ja.actual}
                              </Badge>
                              <Badge variant="outline">
                                {r.payload.model_exposure
                                  ? ja.modelExposure
                                  : ja.noExposure}
                              </Badge>
                            </div>
                            <p className="text-sm leading-7">
                              {r.payload.context.summary}
                            </p>
                            <p className="rounded-lg bg-muted p-3 text-sm leading-6">
                              {ja.selection}: {responseLabel(r.payload)}
                            </p>
                            {r.payload.rationale_explicit && (
                              <p className="whitespace-pre-wrap text-sm leading-6">
                                {ja.notes}: {r.payload.rationale_explicit}
                              </p>
                            )}
                            {r.payload.reversal_conditions.length > 0 && (
                              <p className="text-sm">
                                {ja.reversal}:{" "}
                                {r.payload.reversal_conditions.join(" / ")}
                              </p>
                            )}
                            <p className="break-all text-xs text-muted-foreground">
                              {ja.source}:{" "}
                              {r.payload.source.source_kind === "human_app"
                                ? ja.human
                                : r.payload.source.source_kind}{" "}
                              · {ja.sourceId}: {r.payload.source.artifact_id}
                            </p>
                            <div className="flex flex-wrap gap-2">
                              {r.status === "pending" && (
                                <Button
                                  disabled={busy}
                                  onClick={() =>
                                    void act(async () => {
                                      await command("confirm", {
                                        workspace_id: active,
                                        id: r.id,
                                        expected_revision: r.revision,
                                        request_id: crypto.randomUUID(),
                                      });
                                      setNotice(ja.confirmedMessage);
                                      await refresh();
                                    })
                                  }
                                >
                                  {ja.confirm}
                                </Button>
                              )}
                              <Button
                                variant="outline"
                                disabled={busy}
                                onClick={() => revise(r)}
                              >
                                {ja.revise}
                              </Button>
                              <Button
                                variant="ghost"
                                disabled={busy}
                                onClick={() => setDeleting(r)}
                              >
                                {ja.remove}
                              </Button>
                              <Button
                                variant="ghost"
                                onClick={() => {
                                  sourceRecordId.current = r.id;
                                  setProposal({
                                    ...exposure.current.apply(r.payload, r.id),
                                    request_id: crypto.randomUUID(),
                                  });
                                  setRank(null);
                                  setPage("judge");
                                }}
                              >
                                {ja.rank}
                              </Button>
                            </div>
                            {deleting?.id === r.id && (
                              <div
                                role="alertdialog"
                                aria-label={ja.remove}
                                className="rounded-lg border border-destructive/30 p-4"
                              >
                                <p className="mb-3 text-sm">{ja.deleteHelp}</p>
                                <div className="flex gap-2">
                                  <Button
                                    variant="destructive"
                                    disabled={busy}
                                    onClick={() =>
                                      void act(async () => {
                                        await command("delete", {
                                          workspace_id: active,
                                          id: r.id,
                                          expected_revision: r.revision,
                                        });
                                        setDeleting(null);
                                        if (saved?.id === r.id) setSaved(null);
                                        await refresh();
                                      })
                                    }
                                  >
                                    {ja.deleting}
                                  </Button>
                                  <Button
                                    variant="outline"
                                    onClick={() => setDeleting(null)}
                                  >
                                    {ja.cancel}
                                  </Button>
                                </div>
                              </div>
                            )}
                          </Panel>
                        ))
                    )}
                    <div className="flex justify-between">
                      <Button
                        variant="outline"
                        disabled={offset === 0 || busy}
                        onClick={() => setOffset(Math.max(0, offset - 100))}
                      >
                        {ja.previous}
                      </Button>
                      <Button
                        variant="outline"
                        disabled={memories.length < 100 || busy}
                        onClick={() => setOffset(offset + 100)}
                      >
                        {ja.pagination}
                      </Button>
                    </div>
                  </>
                )}
                {page === "models" && (
                  <>
                    <Panel title={ja.train} description={ja.trainHelp}>
                      <Button
                        className="self-start"
                        disabled={busy || confirmed === 0}
                        onClick={() =>
                          void act(async () => {
                            await command<Model>("train", {
                              workspace_id: active,
                            });
                            await refresh();
                          })
                        }
                      >
                        {busy ? ja.training : ja.train}
                      </Button>
                      <p className="text-xs text-muted-foreground">
                        {ja.modelWarning}
                      </p>
                    </Panel>
                    {!models.data?.models.length ? (
                      <Empty title={ja.noModel} body={ja.noModelBody} />
                    ) : (
                      models.data.models.map((m) => (
                        <Panel
                          key={m.id}
                          title={ja.provisional}
                          description={date(m.created_at)}
                        >
                          <p className="text-xs text-muted-foreground">
                            {m.id}
                          </p>
                          <p className="text-sm">
                            {ja.epoch}: {m.lineage?.length ?? 0}
                            {ja.count}
                          </p>
                          <p className="text-sm">{ja.noAccuracy}</p>
                          <details>
                            <summary className="cursor-pointer text-sm text-primary">
                              {ja.rawEvaluation}
                            </summary>
                            <pre className="mt-3 overflow-auto rounded-lg bg-muted p-4 text-xs">
                              {JSON.stringify(
                                { evaluation: m.evaluation, counts: m.counts },
                                null,
                                2,
                              )}
                            </pre>
                          </details>
                          <Button
                            variant="outline"
                            className="self-start"
                            onClick={() => {
                              setModelId(m.id);
                              setPage("judge");
                            }}
                          >
                            {ja.preview}
                          </Button>
                        </Panel>
                      ))
                    )}
                  </>
                )}
                {page === "map" && (
                  <>
                    <p className="text-sm leading-7 text-muted-foreground">
                      {ja.mapHelp}
                    </p>
                    <div className="grid gap-4 md:grid-cols-2">
                      {gaps.data?.domains.map((g) => (
                        <Panel key={g.domain} title={domains[g.domain]}>
                          <Badge variant="outline">
                            {g.confirmed_records ? ja.recorded : ja.unlearned}
                          </Badge>
                          <p className="text-3xl font-semibold">
                            {g.confirmed_records}
                            <span className="ml-2 text-sm font-normal text-muted-foreground">
                              {ja.count} · {ja.confirmed}
                            </span>
                          </p>
                          <p className="text-xs text-muted-foreground">
                            {ja.unverified}
                          </p>
                        </Panel>
                      ))}
                    </div>
                    {gaps.data?.counts_truncated && (
                      <p className="text-sm">
                        1000{ja.count} / {ja.unverified}
                      </p>
                    )}
                  </>
                )}
                {page === "connections" && (
                  <Empty
                    title={ja.unavailable}
                    body={ja.connectionBody}
                  />
                )}
                {page === "personality" && (
                  staticMode ? (
                    <Empty
                      title={ja.unavailable}
                      body="静的モードでは性格傾向の保存に対応していません。ローカルバックエンドで開いてください。"
                    />
                  ) : (
                    <AssessmentPanel key={active} workspaceId={active} />
                  )
                )}
                {page === "policies" && (
                  <PolicyPanel
                    key={active}
                    workspace={active}
                    proposal={proposal}
                    date={date}
                    onChanged={() => setRank(null)}
                  />
                )}
                {page === "privacy" && (
                  <>
                    <Panel title={nav.privacy} description={ja.privacy}>
                      <Badge variant="outline">{ja.none}</Badge>
                      <Field label={ja.timestamp}>
                        <select
                          className="control"
                          value={zone}
                          onChange={(e) => setZone(e.target.value)}
                        >
                          <option value="local">{ja.localTime}</option>
                          <option value="utc">{ja.utc}</option>
                        </select>
                      </Field>
                      <p className="text-xs text-muted-foreground">
                        {ja.inputOffset}: UTC{" "}
                        {new Date().getTimezoneOffset() <= 0 ? "+" : "−"}
                        {Math.abs(new Date().getTimezoneOffset() / 60)}
                      </p>
                      <div className="flex flex-wrap gap-2">
                        <Button
                          variant="outline"
                          disabled={busy}
                          onClick={() =>
                            void act(async () => {
                              const result = await command<{ jsonl: string }>(
                                "export",
                                { workspace_id: active, offset },
                              );
                              const url = URL.createObjectURL(
                                new Blob([result.jsonl], {
                                  type: "application/x-ndjson",
                                }),
                              );
                              const a = document.createElement("a");
                              a.href = url;
                              a.download = `cdna-records-${offset}.jsonl`;
                              a.click();
                              URL.revokeObjectURL(url);
                            })
                          }
                        >
                          {ja.export} ({offset + 1}–{offset + 100})
                        </Button>
                        <Button
                          variant="destructive"
                          disabled={busy}
                          onClick={() =>
                            void act(async () => {
                              await command("lock");
                              exposure.current.clear();
                              sourceRecordId.current = null;
                              clearSession();
                              client.clear();
                              setProposal(null);
                              setLocked(true);
                            })
                          }
                        >
                          {ja.lock}
                        </Button>
                      </div>
                    </Panel>
                    <Panel title={ja.newWorkspace}>
                      <Field label={ja.workspaceName}>
                        <Input
                          value={workspaceName}
                          maxLength={80}
                          onChange={(e) => setWorkspaceName(e.target.value)}
                        />
                      </Field>
                      <Button
                        className="self-start"
                        disabled={busy || !workspaceName.trim()}
                        onClick={() =>
                          void act(async () => {
                            const r = await command<{
                              id: string;
                              name: string;
                            }>("workspace_create", { name: workspaceName });
                            setNames((n) => ({ ...n, [r.id]: r.name }));
                            setWorkspace(r.id);
                            await client.invalidateQueries({
                              queryKey: ["status"],
                            });
                            setWorkspaceName("");
                          })
                        }
                      >
                        {ja.create}
                      </Button>
                    </Panel>
                  </>
                )}
              </>
            )}
            <footer className="border-t pt-4 text-xs leading-6 text-muted-foreground">
              {ja.local} · {ja.unverified} · {ja.none}
            </footer>
          </div>
        </SidebarInset>
      </SidebarProvider>
    </TooltipProvider>
  );
}
