use corvid_ast::BinaryOp;
use corvid_ir::{IrAgent, IrExpr, IrExprKind, IrLiteral, IrStmt};
use corvid_types::Type;

/// Mirror the Wasm i64 ABI, including conversion of unused parameters before entry.
/// An unrecognized expression keeps the entire handler on the Wasm path.
pub(super) fn method(agent: &IrAgent) -> Option<String> {
    if agent.return_ty != Type::Int || agent.params.iter().any(|p| p.ty != Type::Int) {
        return None;
    }
    let [IrStmt::Return {
        value: Some(value), ..
    }] = agent.body.stmts.as_slice()
    else {
        return None;
    };
    let expression = expression(agent, value)?;
    let params: Vec<_> = (0..agent.params.len()).map(|i| format!("p{i}")).collect();
    let mut body = format!(
        "  [{}]({}) {{\n",
        serde_json::to_string(&agent.name).ok()?,
        params.join(", ")
    );
    for p in params {
        body.push_str(&format!("    {p} = BigInt.asIntN(64, {p});\n"));
    }
    body.push_str(&format!("    return {expression};\n  }}"));
    Some(body)
}

fn expression(agent: &IrAgent, expr: &IrExpr) -> Option<String> {
    match &expr.kind {
        IrExprKind::Literal(IrLiteral::Int(value)) => Some(format!("({value}n)")),
        IrExprKind::Local { local_id, .. } => agent
            .params
            .iter()
            .position(|p| p.local_id == *local_id)
            .map(|i| format!("p{i}")),
        IrExprKind::BinOp { op, left, right } | IrExprKind::WrappingBinOp { op, left, right } => {
            let op = match op {
                BinaryOp::Mul => "*",
                BinaryOp::Add => "+",
                BinaryOp::Sub => "-",
                _ => return None,
            };
            Some(format!(
                "BigInt.asIntN(64, {} {op} {})",
                expression(agent, left)?,
                expression(agent, right)?
            ))
        }
        _ => None,
    }
}
