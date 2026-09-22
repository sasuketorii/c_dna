use cdna_domain::*;
fn main() {
    for (name, schema) in [
        (
            "observation-proposal",
            schemars::schema_for!(ObservationProposal),
        ),
        ("rank-response", schemars::schema_for!(RankResponse)),
        ("policy", schemars::schema_for!(PolicyExpr)),
    ] {
        std::fs::write(
            format!("contracts/{name}.schema.json"),
            serde_json::to_string_pretty(&schema).unwrap() + "\n",
        )
        .unwrap();
    }
}
