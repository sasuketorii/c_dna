<div align="center">
  <img src="docs/assets/c_dna_readme_banner.png" alt="C-DNA — Algorithmic Leadership Engine" width="100%">
</div>

# C-DNA

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](LICENSE)

[日本語](README.md) | **English**

**A decision-learning platform that learns an executive's criteria through dialogue with an LLM, consolidates them into small models, memory and policies, and aims to reproduce that person's judgment at low latency.**

C-DNA is not aiming to be "a chatbot that sounds like an executive." The goal is to **return what the CEO of this specific company would choose — with its grounds and its limits — fast enough to be usable in the middle of a sales conversation or a support call.**

> **Current status: implementation and acceptance testing.** Encrypted local storage, learning and MCP are implemented; profile bootstrap from existing AI conversations and LLM teaching are being integrated. Public playground development is stopped to focus on the engine. Full R1 acceptance, semantic distillation and personal prediction accuracy remain unverified. See the [quickstart](docs/QUICKSTART.md) and [acceptance status](docs/release/ACCEPTANCE.md). Targets below are not claims of achieved performance.

---

## Table of contents

**Part I — Why C-DNA is needed**

- [A company's identity is decided by the CEO's judgment](#a-companys-identity-is-decided-by-the-ceos-judgment)
- [But that judgment never reaches the outer branches](#but-that-judgment-never-reaches-the-outer-branches)
- [Both existing options fall short in the field](#both-existing-options-fall-short-in-the-field)
- [There was no decision algorithm at the right balance point](#there-was-no-decision-algorithm-at-the-right-balance-point)
- [Learning through decisions, not through study](#learning-through-decisions-not-through-study)
- [What C-DNA returns](#what-c-dna-returns)

**Part II — How it is built**

- [Architecture](#architecture)
- [What is actually learned](#what-is-actually-learned)
- [The data model for a single decision](#the-data-model-for-a-single-decision)
- [Learning happens in three stages](#learning-happens-in-three-stages)
- [Technology stack](#technology-stack)
- [Latency target](#latency-target)
- [What "high accuracy" has to mean here](#what-high-accuracy-has-to-mean-here)
- [Privacy](#privacy)
- [Roadmap](#roadmap)
- [Repository layout](#repository-layout)

---

# Part I — Why C-DNA is needed

## A company's identity is decided by the CEO's judgment

What the product should be, which deals to turn down, whether to take quality or the deadline. A company's identity is the accumulation of decisions like these, and the CEO is the one making them.

The problem is that this judgment **is not present at the place and the moment where it is needed.**

The larger a company grows, the smaller the share of decisions the CEO can personally touch. There are days when the CEO simply has no time. And there are situations in the field where decision *speed* itself is the requirement.

## But that judgment never reaches the outer branches

- **A salesperson cannot embody the CEO's identity in the middle of a sales conversation.** Recalling the CEO's criteria and applying them correctly while negotiating terms face to face is not realistic.
- **A support agent cannot embody the CEO's identity in the middle of a complaint call.** The person on the other end will not wait.

Precisely because identity does not reach the far end of every branch, companies are forced into culture programs, values training, and similar activities.

And the assumption behind those programs is eroding. **From here on, talent will be highly fluid.** People will rotate out before years of values training can take hold.

So what is actually needed is this:

> **Even someone who joined today has the CEO's identity available to them.**

## Both existing options fall short in the field

Rules, LLMs and small decision models have different roles. C-DNA puts LLM dialogue at the center of learning, then measures personal agreement and response time when reusing the result. This table describes design roles, not measured superiority over competitors.

| Aspect | Rule-based decision engine | LLM | C-DNA |
| --- | --- | --- | --- |
| **Response speed** | Direct evaluation of defined conditions | Depends on model, input and connection | Reuse local learning results; measure input-to-answer latency (see [Latency target](#latency-target)) |
| **Accuracy** | Depends on rules and their scope | Depends on model, personal context and task | Measure personal agreement and coverage; ask or abstain outside validated scope |
| **Combined conditions** | Weak. Every new combination means another rule | Strong | Learns combinations such as "short deadline × outsource" |
| **Setup cost** | Define and maintain rules | May reuse existing conversations and memory | Import existing AI understanding with a dedicated prompt, then ask about gaps |
| **Running cost** | Depends on rules and runtime | Depends on model, subscription and usage | Optimize LLM training costs separately from decision-time compute |
| **Grounds for a decision** | Which rule matched | Generated prose; the reasoning tends to be post-hoc | Past decisions, situations, and exception conditions, returned with citations |
| **Whose judgment is it** | Depends on the rules' provenance | Depends on personal information, conversation and instructions | **Independently evaluate agreement with this company's CEO** |

The LLM proposes hypotheses about criteria the person has not yet articulated and asks about reasons, counterexamples and reversal conditions. Human corrections guide representation and training improvements; independent human answers test what can enter the decision runtime.

## There was no decision algorithm at the right balance point

This is where C-DNA starts.

The goal is to **return this company's CEO's judgment at field speed**.

Learning uses LLMs to deepen understanding. Inference reuses learned representations, small models and memory. New text and unfamiliar concepts may need additional interpretation; measurements must include that work, rather than presenting structured-candidate scoring alone as end-to-end speed (see [Architecture](#architecture)).

## Learning through decisions, not through study

An LLM is large enough that a person **can** study the CEO's reasoning as long-form text. But during all the hours spent doing that, the employee is held at a high cognitive load. That leads directly to **burnout.**

C-DNA inverts the order. **It learns the executive's tendencies instantly, through the decisions being made in the field.**

Retention is in fact higher when something is **experienced** than when it is studied. With C-DNA this asymmetry pays off in both directions.

| | What C-DNA provides |
| --- | --- |
| **The executive** | Learns their own decision logic — the part they had never put into words |
| **The field** | Experiences the CEO's judgment as an actual decision, instead of memorizing it in a classroom |

Two things follow:

> **The CEO can clone their judgment.**
> **The field can use the CEO's judgment directly.**

The intended experience depends on a continuous loop in which the person and the LLM examine disagreements and update what the engine has learned.

## What C-DNA returns

"Faithful to the person" and "good for the business" are not the same metric. Blending them produces something useless for both purposes. So C-DNA's output is separated from the start.

| Mode | What it returns |
| --- | --- |
| **Reproduction** | "Given who you are now, you would likely choose this one." |
| **Suggestion** | "This differs from your preference, but under these conditions it deserves consideration." |
| **Hold / ask** | "Without this information, your judgment cannot be reproduced." |

The third mode matters most. **Refusing to answer when the inputs are insufficient** is treated as seriously as accuracy itself.

---

# Part II — How it is built

## Architecture

The central loop is **CEO ↔ LLM ↔ learning engine**. The LLM conducts detailed teaching dialogue; Python trains and evaluates representations and small models; Rust and Mojo use the resulting artifacts for fast decisions. Separating stages manages latency and authority; it does not remove the LLM from learning.

Bootstrap starts by pasting a dedicated prompt into the person's existing ChatGPT or Claude conversation and importing what it can actually access about their judgments. The person corrects the profile and the LLM asks about important gaps and exceptions. The initial 17 features are a comparison baseline, not a permanent limit on the person's concepts.

```text
Dedicated prompt → ChatGPT / Claude judgment profile and exceptions
Conversation exports and human corrections
Real decisions made in Codex / Claude Code
Four-choice, pairwise, and free-text input in the C-DNA app
                    ↓
        Extract and structure decision candidates
                    ↓
    Verify sources / deduplicate / confirm with the person
                    ↓
            Local decision database
                    ↕
     CEO ↔ LLM: reasons, counterexamples, reversal conditions
     Next questions / representation and training hypotheses
                    ↓
       ┌────────────┴─────────────┐
       ↓                          ↓
 Decision memory &            Training pipeline
 explicit rules               and independent evaluation (Python)
 applied immediately              ↓
       ↓                    Versioned representation + small model
 Similar-case retrieval            │
       └────────────┬─────────────┘
                    ↓
        Rust-centered fast decision runtime
        Numeric kernels in Mojo where they pay off
                    ↓
     Ranked candidates / grounds / missing inputs
     Whether the system should decline to answer
                    ↓
     Consumed by C-DNA, Codex, Claude Code,
     and other working agents
```

External agents connect through an MCP server as the shared entry point. C-DNA exposes tools for retrieving decision policy, searching past decisions, registering observed decisions as candidates, predicting a choice, listing under-learned areas, and submitting answers. **No agent may set a record to "confirmed by the person" on its own.** Confirmation is bound to trusted input paths such as direct app interaction.

## What is actually learned

Pressing "an executive's DNA" into a single blob mixes personality, company circumstances, and management policy together. The same person decides differently when cash is abundant than when cash flow is the priority. A model that reads that as a change in personality is not what we want.

So C-DNA keeps six layers apart.

| Layer | What it holds | Examples |
| --- | --- | --- |
| **Personality traits** | Relatively stable behavioral tendencies | Openness to new things, caution, planfulness |
| **Values and priorities** | What the person weighs most | Growth, profit, time, quality, trust, autonomy |
| **Explicit policy** | Policies and constraints the person has approved | Never take deals under condition X; amounts above Y need my sign-off |
| **Company / deal context** | Conditions at the time of the decision | Cash, headcount, deadline, business stage, impact on existing customers |
| **Revealed preference** | What they choose *under these conditions* | Outsourcing over in-house, continuity over short-term revenue |
| **Outcome and review** | What happened after the choice | Profit, effort, failure, regret, revision of criteria |

**Revealed preference is the lead actor. Personality assessment is a supporting input.** "Openness is high, therefore approve this investment" is not a valid inference. If a personality profile is essentially fixed, it cannot by itself explain decisions that change with daily conditions.

## The data model for a single decision

C-DNA's strength does not come from storing large volumes of text. It comes from **recording, in comparable form, why that candidate was chosen in that situation.** Each decision carries its situation, objective, constraints, options, outcome, the person's stated reasons, and its source and confirmation status.

Three fields in particular are mandatory.

**What would have to change to flip the decision.** "Chose to outsource" is weak on its own. "Outsourced because the deadline mattered this time; in-house if the deadline were longer and the goal were building internal assets" reveals the boundary of the judgment.

**Whether an AI suggestion preceded the decision.** Choosing independently and accepting an AI recommendation are both useful signals, but they are not the same observation.

**What was unknown at the time.** Revenue and failures discovered later are never mixed back into the inputs available at decision time. This prevents the system from learning hindsight explanations and appearing to predict the future.

## Learning happens in three stages

Not everything needs retraining after every answer. **"Remembering immediately" and "changing model weights immediately" are not the same thing.**

| Stage | What happens after an answer | Purpose |
| --- | --- | --- |
| **Immediate** | Append to decision memory and approved policy | The same decision is retrievable right away |
| **Incremental** | Partial-fit the small comparison model | Update preferences with minimal computation |
| **Evaluated** | Retrain the stronger model on accumulated data | Promote to production only after quality checks |

Even replying "there is a record of the person choosing this under the same conditions" immediately after a single answer already delivers learning value. Memory updates fast; model promotion stays conservative.

The decision model is deliberately **not a four-class classifier**, because option A means something different in every question. What it learns is a function returning how strongly the person would prefer a given option, from the situation and the option's content. It starts as a regularized comparison model and moves to a candidate-ranking model as data accumulates.

## Technology stack

| Area | Choice | Role |
| --- | --- | --- |
| **Training** | Python (scikit-learn / LightGBM / Polars) | Small comparison model, ranking model, shaping and evaluating decision data |
| **App, data, inference control** | Rust (Tauri / rusqlite / MCP SDK / ONNX Runtime bindings) | Desktop app, local decision DB, MCP server, inference execution |
| **Numeric acceleration** | Mojo | Feature transforms, batch scoring of candidates, similarity — only where it pays off |
| **Environment and data contracts** | uv / Pydantic | Pinned Python environments, schemas for decision events and APIs |

Rather than pushing everything into Mojo from the start, the order is: establish model quality with existing libraries, then accelerate that model. **We do not assume that an arbitrary model trained in Python can be converted to Mojo automatically.**

Inference therefore sits behind a swappable contract (`DecisionScorer`), so a small Rust scorer, native tree inference, ONNX Runtime, and Mojo can be compared against each other. **C-DNA's value must not depend on whether Mojo is adopted.** Mojo supports macOS and Linux natively and Windows via WSL, so the product always stays usable through the standard Rust inference path on machines where Mojo does not run.

## Latency target

The initial design target is:

> **p95 under 10 ms for comparing a handful to a few dozen candidates, given structured input, a loaded model, and local execution.**

**This is a target, not a measurement.** The following are excluded from it and measured separately:

| Stage | Why it is measured separately |
| --- | --- |
| Parsing new text | Depends on the LLM and the text volume |
| Embedding generation | Depends on the model and input length |
| DB reads and network | Wait time unrelated to numeric computation |
| Candidate scoring | The actual Rust / Mojo comparison target |
| Explanation generation | A different latency profile when a generative LLM is used |

**"The model itself took 1 ms" will never be presented as the application's decision latency.**

## What "high accuracy" has to mean here

Looking at a single "accuracy" number is dangerous. C-DNA evaluates these separately:

- **Agreement with the person** — how well unseen situations reproduce their actual choice
- **Recall of acceptable options** — whether options the person would also accept are being dropped
- **Absence of overconfidence** — whether high-confidence answers actually agree
- **Appropriate abstention** — whether it forces an answer under missing information or in unknown territory
- **Coverage** — how many situations it can handle while sustaining high agreement
- **Burden on the person** — improvement relative to the time they spent teaching it
- **Speed and resource use** — end-to-end latency, memory, CPU

Baselines include at minimum: explicit rules only, similar-case retrieval only, an LLM with a profile of the person, and C-DNA's small model. Evaluation data is split by training on the past and evaluating on the future, and paraphrases of the same case are never split across training and evaluation.

Also, even when the person's choice is predicted correctly, it does not yet follow that "this choice produced more profit than the one not taken." **Accumulating outcome data and identifying causally better decisions are treated as separate problems.**

## Privacy

The design assumes **the operator never sees the text of a decision.** Analysis runs on the user's device, and only permitted diagnostic results leave it.

| Data | Default |
| --- | --- |
| Conversation text, notes, company names, customer names | Not transmitted |
| Original text of past decisions | Not transmitted |
| Embedding vectors, model weights, gradients | Not transmitted |
| Processing time, memory usage, error categories | Shared with consent |
| Record counts, missing-input rates, per-category evaluation | Shared with small-count suppression |

**"It's a vector, so it's anonymous" and "it's a trained model, so it's safe" are not accepted as reasoning.** Core functionality keeps working with data sharing turned off. And note that even in a local-first app, sending text to a cloud LLM means that processing happens externally: **"not sent to the operator" and "never leaves the device" are kept as separate settings.**

## Roadmap

| Stage | Scope to complete |
| --- | --- |
| **1. Collect decisions and use them with citations** | Summary import → confirmation by the person → four-choice / pairwise / notes → local storage → read and write over MCP. Useful for retrieving past decisions without waiting for any model |
| **2. Actually learn, and ask for what is missing** | Introduce the small comparison model and benchmark it against the ranking model. Detect thin areas, ask about them, and verify that answers improve reproduction on held-out cases |
| **3. Finish speed and distribution quality** | Move inference to Rust and add Mojo where measurements justify it. Verify that identical inputs and models yield identical scores and abstentions across implementations |
| **4. A privacy-preserving improvement loop** | Local diagnosis, improvement proposals, a preview of what would be shared, consent-based metrics, and approval-gated model promotion |

Stage 2 is the point at which **"teaching it changes its judgment" becomes a measurable experience.**

## Repository layout

```text
c-dna/
├── README.md         # Japanese
├── README.en.md      # English (this file)
├── LICENSE
├── .gitignore
├── exec_plan/        # Requirements and execution plans
└── docs/
    ├── assets/       # Images used by the READMEs
    └── internal/     # Private design notes (gitignored)
```

Implementation directories will be added when roadmap stage 1 begins. Directories that do not exist are not listed as if they did.

---

## What is genuinely ours to build

Machine learning itself, local inference, MCP, and the desktop app foundation can largely be borrowed from existing libraries. Three things are ours to build:

1. **A data model that accumulates the person's decisions with sources, context, and exception conditions.**
2. **A questioning, comparison, and re-confirmation loop that closes gaps with minimal answering time.**
3. **A fast decision interface, usable from existing agents, that returns both its grounds and its limits.**

With those three in place, C-DNA stops being "a chat clone of an executive."

**You think in ChatGPT or Claude. You execute in Codex or Claude Code. The judgments that surface along the way are learned by C-DNA, so that on the next job the policy is already available without the person explaining it again.**

That loop is what C-DNA is aiming for.

---

## License

[GNU Affero General Public License v3.0](LICENSE) © 2026 sasuketorii

Under AGPL-3.0, offering a modified version of C-DNA to users over a network also obliges you to provide the source code of that modified version.
