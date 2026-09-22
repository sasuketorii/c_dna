# UI provenance and distribution

C-DNA is distributed under AGPL. Paid Shadcnblocks source and derivatives cannot be published in a public repository under its [license](https://www.shadcnblocks.com/license). The initially considered licensed admin shell was replaced before delivery. No proprietary template source is included.

| Region | Public source | Application adaptation |
| --- | --- | --- |
| Sidebar and inset shell | [shadcn sidebar-07](https://ui.shadcn.com/blocks/sidebar), base-vega registry | Japanese navigation, workspace selector, product title, local status |
| Card, button, fields, badge, empty, tooltip, sheet | [shadcn/ui](https://github.com/shadcn-ui/ui), base-vega registry | Japanese content and engine data bindings |
| Product screens | Application composition of the above MIT primitives | C-DNA domain contract and required answer flows |

Registry installation used exact CLI shadcn 4.21.0: view, add --dry-run, add. Components use Base UI, not Radix. MIT attribution is preserved in SHADCN-LICENSE.md. The sidebar cookie write is removed; its keyboard shortcut ignores IME, repeat and text inputs. Light/dark themes follow the explicit product requirement. No external images, fonts or tracking resources load at runtime.

This is source provenance, not visual acceptance. TypeScript and production build verification is separate from browser/device verification. Native Tauri packaging is not supplied by this browser entry point.
