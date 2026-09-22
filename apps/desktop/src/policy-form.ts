import { attributes, facts, policyJa } from "./ja.ts";
export type Comparison = "gte" | "lte" | "gt" | "lt" | "eq";
export type Expr =
  | {
      op: "compare";
      field: string;
      comparison: Comparison;
      value: unknown;
      unit: string | null;
    }
  | { op: "all" | "any"; args: Expr[] }
  | { op: "not"; arg: Expr }
  | { op: "exists"; field: string }
  | { op: "in"; field: string; values: unknown[]; unit: string | null };
export type Policy = {
  statement: string;
  when: Expr;
  requires: Expr;
  effective_from: string;
  expires_at: string | null;
};
export type PolicyForm = {
  statement: string;
  whenField: string;
  whenComparison: Comparison;
  whenValue: number;
  requiresField: string;
  requiresComparison: Comparison;
  requiresValue: number;
  expires: string;
};
export function buildPolicy(form: PolicyForm, now = new Date()): Policy {
  const valid = (n: number) => Number.isFinite(n) && n >= 0 && n <= 1;
  const expiry = form.expires ? new Date(form.expires) : null;
  if (
    !form.statement.trim() ||
    new TextEncoder().encode(form.statement).length > 4096 ||
    !valid(form.whenValue) ||
    !valid(form.requiresValue) ||
    !facts[form.whenField] ||
    !attributes[form.requiresField] ||
    (expiry && (!Number.isFinite(expiry.getTime()) || expiry <= now))
  )
    throw new Error(policyJa.invalid);
  return {
    statement: form.statement.trim(),
    when: {
      op: "compare",
      field: form.whenField,
      comparison: form.whenComparison,
      value: form.whenValue,
      unit: "ratio",
    },
    requires: {
      op: "compare",
      field: form.requiresField,
      comparison: form.requiresComparison,
      value: form.requiresValue,
      unit: "ratio",
    },
    effective_from: now.toISOString(),
    expires_at: expiry?.toISOString() ?? null,
  };
}
