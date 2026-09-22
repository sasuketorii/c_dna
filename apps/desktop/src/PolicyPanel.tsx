import { useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { command, rankRequest, staticMode } from "./api";
import type { Proposal } from "./api";
import {
  attributes,
  facts,
  ja,
  policyJa as p,
  policyComparisons,
  policyResults,
} from "./ja";
import { buildPolicy } from "./policy-form";
import type { Comparison, Expr, Policy, PolicyForm } from "./policy-form";
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
  Empty,
  EmptyHeader,
  EmptyTitle,
  EmptyDescription,
} from "./components/ui/empty";

type Document = {
  id: string;
  revision: number;
  payload: { policy: Policy; status: string };
};
type Checks = {
  candidates: {
    candidate_id: string;
    status: string;
    checks: { policy_id: string; statement: string; status: string }[];
  }[];
};
const initial: PolicyForm = {
  statement: "",
  whenField: "deadline_pressure",
  whenComparison: "gte",
  whenValue: 0.7,
  requiresField: "speed",
  requiresComparison: "gte",
  requiresValue: 0.7,
  expires: "",
};
function expression(expr: Expr): string {
  switch (expr.op) {
    case "all":
    case "any":
      return `${expr.op === "all" ? p.all : p.any}（${expr.args.map(expression).join(" / ")}）`;
    case "not":
      return `${p.not}（${expression(expr.arg)}）`;
    case "exists":
      return `${facts[expr.field] || attributes[expr.field] || expr.field}：${p.exists}`;
    case "in":
      return `${facts[expr.field] || attributes[expr.field] || expr.field}：${p.included} ${expr.values.map(String).join(" / ")}`;
    case "compare":
      return `${facts[expr.field] || attributes[expr.field] || expr.field} ${String(expr.value)} ${policyComparisons[expr.comparison]}${expr.unit ? ` (${expr.unit === "ratio" ? p.units : expr.unit})` : ""}`;
  }
}
export function PolicyPanel({
  workspace,
  proposal,
  date,
  onChanged,
}: {
  workspace: string;
  proposal: Proposal | null;
  date: (value: string) => string;
  onChanged: () => void;
}) {
  const client = useQueryClient();
  const [form, setForm] = useState<PolicyForm>(initial);
  const [busy, setBusy] = useState(false);
  const guard = useRef(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [reviewed, setReviewed] = useState<Record<string, number>>({});
  const [remove, setRemove] = useState<string | null>(null);
  const [checks, setChecks] = useState<Checks | null>(null);
  const query = useQuery({
    queryKey: ["policies", workspace],
    queryFn: () =>
      command<{ policies: Document[] }>("policies", {
        workspace_id: workspace,
      }),
    enabled: !staticMode,
  });
  async function run(action: () => Promise<void>) {
    if (guard.current) return;
    guard.current = true;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      guard.current = false;
      setBusy(false);
    }
  }
  async function refresh() {
    setChecks(null);
    onChanged();
    await client.invalidateQueries({ queryKey: ["models"] });
    await client.invalidateQueries({ queryKey: ["policies", workspace] });
  }
  if (staticMode)
    return (
      <Empty className="border">
        <EmptyHeader>
          <EmptyTitle>{ja.unavailable}</EmptyTitle>
          <EmptyDescription>{p.staticUnavailable}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  function status(doc: Document) {
    if (doc.payload.status !== "approved") return p.draft;
    const policy = doc.payload.policy;
    return Date.now() < Date.parse(policy.effective_from)
      ? p.scheduled
      : policy.expires_at && Date.now() >= Date.parse(policy.expires_at)
        ? p.expired
        : p.approved;
  }
  return (
    <div className="flex flex-col gap-6">
      <p className="text-sm leading-7 text-muted-foreground">{p.intro}</p>
      {(error || query.error) && (
        <p
          role="alert"
          className="rounded-lg border border-destructive/30 p-3 text-sm"
        >
          {error || (query.error as Error).message}
        </p>
      )}
      {notice && (
        <p role="status" className="rounded-lg border p-3 text-sm">
          {notice}
        </p>
      )}
      <Card>
        <CardHeader>
          <CardTitle>{p.newPolicy}</CardTitle>
          <CardDescription>{p.ratioHelp}</CardDescription>
        </CardHeader>
        <CardContent>
          <form
            className="flex flex-col gap-5"
            onSubmit={(e) => {
              e.preventDefault();
              void run(async () => {
                const policy = buildPolicy(form);
                await command("propose_policy", {
                  workspace_id: workspace,
                  policy,
                });
                setForm(initial);
                setNotice(p.draftSaved);
                await refresh();
              });
            }}
          >
            <fieldset disabled={busy} className="flex flex-col gap-5">
              <label className="flex flex-col gap-2 text-sm">
                {p.statement}
                <Textarea
                  required
                  maxLength={1000}
                  value={form.statement}
                  placeholder={p.statementPlaceholder}
                  onChange={(e) =>
                    setForm({ ...form, statement: e.target.value })
                  }
                />
              </label>
              {(["when", "requires"] as const).map((kind) => (
                <fieldset
                  key={kind}
                  className="grid gap-3 rounded-lg border p-4 sm:grid-cols-3"
                >
                  <legend className="px-2 text-sm font-medium">
                    {kind === "when" ? p.when : p.requires}
                  </legend>
                  <label className="flex flex-col gap-2 text-sm">
                    {kind === "when" ? ja.background : ja.attributes}
                    <select
                      className="control"
                      value={
                        kind === "when" ? form.whenField : form.requiresField
                      }
                      onChange={(e) =>
                        setForm({
                          ...form,
                          [kind === "when" ? "whenField" : "requiresField"]:
                            e.target.value,
                        })
                      }
                    >
                      {Object.entries(kind === "when" ? facts : attributes).map(
                        ([key, label]) => (
                          <option key={key} value={key}>
                            {label}
                          </option>
                        ),
                      )}
                    </select>
                  </label>
                  <label className="flex flex-col gap-2 text-sm">
                    {p.threshold}
                    <Input
                      required
                      type="number"
                      min={0}
                      max={1}
                      step={0.01}
                      value={
                        kind === "when" ? form.whenValue : form.requiresValue
                      }
                      onChange={(e) =>
                        setForm({
                          ...form,
                          [kind === "when" ? "whenValue" : "requiresValue"]:
                            e.target.valueAsNumber,
                        })
                      }
                    />
                  </label>
                  <label className="flex flex-col gap-2 text-sm">
                    {p.comparison}
                    <select
                      className="control"
                      value={
                        kind === "when"
                          ? form.whenComparison
                          : form.requiresComparison
                      }
                      onChange={(e) =>
                        setForm({
                          ...form,
                          [kind === "when"
                            ? "whenComparison"
                            : "requiresComparison"]: e.target
                            .value as Comparison,
                        })
                      }
                    >
                      {Object.entries(policyComparisons).map(([key, label]) => (
                        <option key={key} value={key}>
                          {label}
                        </option>
                      ))}
                    </select>
                  </label>
                </fieldset>
              ))}
              <label className="flex flex-col gap-2 text-sm">
                {p.expires}
                <Input
                  type="datetime-local"
                  value={form.expires}
                  onChange={(e) =>
                    setForm({ ...form, expires: e.target.value })
                  }
                />
              </label>
              <p className="text-xs text-muted-foreground">
                {p.effective}：{p.now}
              </p>
              <Button
                type="submit"
                className="self-start"
                disabled={!form.statement.trim()}
              >
                {busy ? ja.wait : p.saveDraft}
              </Button>
            </fieldset>
          </form>
        </CardContent>
      </Card>
      <div className="flex flex-wrap gap-3">
        <Button
          variant="outline"
          disabled={busy}
          onClick={() => void run(refresh)}
        >
          {p.refresh}
        </Button>
        <Button
          variant="outline"
          disabled={busy || !proposal}
          onClick={() =>
            void run(async () => {
              if (proposal)
                setChecks(
                  await command<Checks>("check_policy", {
                    request: rankRequest(proposal),
                  }),
                );
            })
          }
        >
          {p.check}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">{p.checkHelp}</p>
      {query.isPending && <p role="status">{ja.loading}</p>}
      {query.data?.policies.length === 0 && (
        <Empty className="border">
          <EmptyHeader>
            <EmptyTitle>{p.empty}</EmptyTitle>
            <EmptyDescription>{p.emptyBody}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}
      {query.data?.policies.map((doc) => (
        <Card key={doc.id}>
          <CardHeader>
            <CardTitle>{doc.payload.policy.statement}</CardTitle>
            <CardDescription>
              {ja.revision} {doc.revision}
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Badge variant="outline">{status(doc)}</Badge>
            <details>
              <summary className="cursor-pointer text-sm text-primary">
                {p.details} · {p.readOnly}
              </summary>
              <dl className="mt-4 flex flex-col gap-3 text-sm">
                <div>
                  <dt className="text-muted-foreground">{p.when}</dt>
                  <dd>{expression(doc.payload.policy.when)}</dd>
                </div>
                <div>
                  <dt className="text-muted-foreground">{p.requires}</dt>
                  <dd>{expression(doc.payload.policy.requires)}</dd>
                </div>
                <div>
                  <dt className="text-muted-foreground">{p.effective}</dt>
                  <dd>{date(doc.payload.policy.effective_from)}</dd>
                </div>
                <div>
                  <dt className="text-muted-foreground">{p.expires}</dt>
                  <dd>
                    {doc.payload.policy.expires_at
                      ? date(doc.payload.policy.expires_at)
                      : p.noExpiry}
                  </dd>
                </div>
              </dl>
              <p className="mt-3 text-xs text-muted-foreground">
                {p.readOnlyHelp}
              </p>
              {doc.payload.status === "draft" && (
                <div className="mt-4 flex flex-col gap-3">
                  <label className="flex items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      disabled={busy}
                      checked={reviewed[doc.id] === doc.revision}
                      onChange={(e) =>
                        setReviewed({
                          ...reviewed,
                          [doc.id]: e.target.checked ? doc.revision : -1,
                        })
                      }
                    />
                    {p.acknowledge}
                  </label>
                  <Button
                    className="self-start"
                    disabled={busy || reviewed[doc.id] !== doc.revision}
                    onClick={() =>
                      void run(async () => {
                        await command("approve_policy", {
                          workspace_id: workspace,
                          id: doc.id,
                          expected_revision: doc.revision,
                        });
                        setNotice(p.approvalDone);
                        await refresh();
                      })
                    }
                  >
                    {p.approve}
                  </Button>
                </div>
              )}
            </details>
            {doc.payload.status === "draft" && (
              <p className="text-xs text-muted-foreground">{p.noApproval}</p>
            )}
            <Button
              className="self-start"
              variant="ghost"
              disabled={busy}
              onClick={() => setRemove(doc.id)}
            >
              {p.revoke}
            </Button>
            {remove === doc.id && (
              <div role="alert" className="rounded-lg border p-3">
                <p className="mb-3 text-sm">{p.revokeConfirm}</p>
                <div className="flex gap-2">
                  <Button
                    variant="destructive"
                    disabled={busy}
                    onClick={() =>
                      void run(async () => {
                        await command("revoke_policy", {
                          workspace_id: workspace,
                          id: doc.id,
                          expected_revision: doc.revision,
                        });
                        setRemove(null);
                        setNotice(p.revokeDone);
                        await refresh();
                      })
                    }
                  >
                    {p.revoke}
                  </Button>
                  <Button
                    variant="outline"
                    disabled={busy}
                    onClick={() => setRemove(null)}
                  >
                    {ja.cancel}
                  </Button>
                </div>
              </div>
            )}
          </CardContent>
        </Card>
      ))}
      {checks && (
        <Card>
          <CardHeader>
            <CardTitle>{p.checks}</CardTitle>
            <CardDescription>{ja.none}</CardDescription>
          </CardHeader>
          <CardContent>
            {checks.candidates.map((row) => (
              <div key={row.candidate_id} className="rounded-lg border p-4">
                <p className="text-sm leading-7">
                  {proposal?.candidates.find((c) => c.id === row.candidate_id)
                    ?.text || row.candidate_id}
                </p>
                <Badge className="my-2" variant="secondary">
                  {policyResults[row.status] || row.status}
                </Badge>
                {row.checks.map((check) => (
                  <p
                    key={check.policy_id}
                    className="text-xs leading-6 text-muted-foreground"
                  >
                    {check.statement}：
                    {policyResults[check.status] || check.status}
                  </p>
                ))}
              </div>
            ))}
          </CardContent>
        </Card>
      )}
    </div>
  );
}
