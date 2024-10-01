use std::{
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};

use eframe::{App, CreationContext};
use egui::{Color32, Id, Ui};
use egui_snarl::{
    ui::{AnyPins, PinInfo, SnarlStyle, SnarlViewer, WireStyle},
    InPin, InPinId, NodeId, OutPin, OutPinId, Snarl,
};
use layout::{
    backends::svg::SVGWriter,
    core::format::Visible,
    gv::{DotParser, GraphBuilder},
    std_shapes::shapes::{Element, ShapeKind},
    topo::{layout::VisualGraph, placer::place::Placer},
};

const STRING_COLOR: Color32 = Color32::from_rgb(0x00, 0xb0, 0x00);
const NUMBER_COLOR: Color32 = Color32::from_rgb(0xb0, 0x00, 0x00);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Error parsing DOT graph: [{0}]")]
    DotParserError(String),
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum Node {
    /// Value node with a single output.
    Named(String),

    /// Expression node with a single output.
    /// It has number of inputs equal to number of variables in the expression.
    ExprNode(ExprNode),
}

impl Node {
    fn name(&self) -> &str {
        match self {
            Node::Named(_) => "String",
            Node::ExprNode(_) => "ExprNode",
        }
    }

    fn number_out(&self) -> f64 {
        match self {
            Node::ExprNode(expr_node) => expr_node.eval(),
            _ => unreachable!(),
        }
    }

    fn number_in(&mut self, idx: usize) -> &mut f64 {
        match self {
            Node::ExprNode(expr_node) => &mut expr_node.values[idx - 1],
            _ => unreachable!(),
        }
    }

    fn label_in(&mut self, idx: usize) -> &str {
        match self {
            Node::ExprNode(expr_node) => &expr_node.bindings[idx - 1],
            _ => unreachable!(),
        }
    }

    fn string_out(&self) -> &str {
        match self {
            Node::Named(value) => value,
            _ => unreachable!(),
        }
    }

    fn string_in(&mut self) -> &mut String {
        match self {
            Node::ExprNode(expr_node) => &mut expr_node.text,
            _ => unreachable!(),
        }
    }

    fn expr_node(&mut self) -> &mut ExprNode {
        match self {
            Node::ExprNode(expr_node) => expr_node,
            _ => unreachable!(),
        }
    }
}

struct DemoViewer;

impl SnarlViewer<Node> for DemoViewer {
    #[inline]
    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<Node>) {
        // Validate connection
        match (&snarl[from.id.node], &snarl[to.id.node]) {
            (_, Node::Named(_)) => {
                unreachable!("String node has no inputs")
            }
            (Node::ExprNode(_), Node::ExprNode(_)) if to.id.input == 0 => {
                return;
            }
            (Node::ExprNode(_), Node::ExprNode(_)) => {}
            (Node::Named(_), Node::ExprNode(_)) if to.id.input == 0 => {}
            (Node::Named(_), Node::ExprNode(_)) => {
                return;
            }
        }

        for &remote in &to.remotes {
            snarl.disconnect(remote, to.id);
        }

        snarl.connect(from.id, to.id);
    }

    fn title(&mut self, node: &Node) -> String {
        match node {
            Node::Named(_) => "String".to_owned(),
            Node::ExprNode(_) => "Expr".to_owned(),
        }
    }

    fn inputs(&mut self, node: &Node) -> usize {
        match node {
            Node::Named(_) => 0,
            Node::ExprNode(expr_node) => 1 + expr_node.bindings.len(),
        }
    }

    fn outputs(&mut self, node: &Node) -> usize {
        match node {
            Node::Named(_) => 1,
            Node::ExprNode(_) => 1,
        }
    }

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut Ui,
        _scale: f32,
        snarl: &mut Snarl<Node>,
    ) -> PinInfo {
        match snarl[pin.id.node] {
            Node::Named(_) => {
                unreachable!("String node has no inputs")
            }
            Node::ExprNode(_) if pin.id.input == 0 => {
                let changed = match &*pin.remotes {
                    [] => {
                        let input = snarl[pin.id.node].string_in();
                        let r = egui::TextEdit::singleline(input)
                            .clip_text(false)
                            .desired_width(0.0)
                            .margin(ui.spacing().item_spacing)
                            .show(ui)
                            .response;

                        r.changed()
                    }
                    [remote] => {
                        let new_string = snarl[remote.node].string_out().to_owned();

                        egui::TextEdit::singleline(&mut &*new_string)
                            .clip_text(false)
                            .desired_width(0.0)
                            .margin(ui.spacing().item_spacing)
                            .show(ui);

                        let input = snarl[pin.id.node].string_in();
                        if new_string != *input {
                            *input = new_string;
                            true
                        } else {
                            false
                        }
                    }
                    _ => unreachable!("Expr pins has only one wire"),
                };

                if changed {
                    let expr_node = snarl[pin.id.node].expr_node();

                    if let Ok(expr) = syn::parse_str(&expr_node.text) {
                        expr_node.expr = expr;

                        let values = Iterator::zip(
                            expr_node.bindings.iter().map(String::clone),
                            expr_node.values.iter().copied(),
                        )
                        .collect::<HashMap<String, f64>>();

                        let mut new_bindings = Vec::new();
                        expr_node.expr.extend_bindings(&mut new_bindings);

                        let old_bindings =
                            std::mem::replace(&mut expr_node.bindings, new_bindings.clone());

                        let new_values = new_bindings
                            .iter()
                            .map(|name| values.get(&**name).copied().unwrap_or(0.0))
                            .collect::<Vec<_>>();

                        expr_node.values = new_values;

                        let old_inputs = (0..old_bindings.len())
                            .map(|idx| {
                                snarl.in_pin(InPinId {
                                    node: pin.id.node,
                                    input: idx + 1,
                                })
                            })
                            .collect::<Vec<_>>();

                        for (idx, name) in old_bindings.iter().enumerate() {
                            let new_idx =
                                new_bindings.iter().position(|new_name| *new_name == *name);

                            match new_idx {
                                None => {
                                    snarl.drop_inputs(old_inputs[idx].id);
                                }
                                Some(new_idx) if new_idx != idx => {
                                    let new_in_pin = InPinId {
                                        node: pin.id.node,
                                        input: new_idx,
                                    };
                                    for &remote in &old_inputs[idx].remotes {
                                        snarl.disconnect(remote, old_inputs[idx].id);
                                        snarl.connect(remote, new_in_pin);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                PinInfo::triangle().with_fill(STRING_COLOR).with_wire_style(
                    WireStyle::AxisAligned {
                        corner_radius: 10.0,
                    },
                )
            }
            Node::ExprNode(ref expr_node) => {
                if pin.id.input <= expr_node.bindings.len() {
                    match &*pin.remotes {
                        [] => {
                            let node = &mut snarl[pin.id.node];
                            ui.label(node.label_in(pin.id.input));
                            ui.add(egui::DragValue::new(node.number_in(pin.id.input)));
                            PinInfo::square().with_fill(NUMBER_COLOR)
                        }
                        [remote] => {
                            let new_value = snarl[remote.node].number_out();
                            let node = &mut snarl[pin.id.node];
                            ui.label(node.label_in(pin.id.input));
                            ui.label(format_float(new_value));
                            *node.number_in(pin.id.input) = new_value;
                            PinInfo::square().with_fill(NUMBER_COLOR)
                        }
                        _ => unreachable!("Expr pins has only one wire"),
                    }
                } else {
                    ui.label("Removed");
                    PinInfo::circle().with_fill(Color32::BLACK)
                }
            }
        }
    }

    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut Ui,
        _scale: f32,
        snarl: &mut Snarl<Node>,
    ) -> PinInfo {
        match snarl[pin.id.node] {
            Node::Named(ref mut value) => {
                assert_eq!(pin.id.output, 0, "String node has only one output");
                let edit = egui::TextEdit::singleline(value)
                    .clip_text(false)
                    .desired_width(0.0)
                    .margin(ui.spacing().item_spacing);
                ui.add(edit);
                PinInfo::triangle().with_fill(STRING_COLOR).with_wire_style(
                    WireStyle::AxisAligned {
                        corner_radius: 10.0,
                    },
                )
            }
            Node::ExprNode(ref expr_node) => {
                let value = expr_node.eval();
                assert_eq!(pin.id.output, 0, "Expr node has only one output");
                ui.label(format_float(value));
                PinInfo::square().with_fill(NUMBER_COLOR)
            }
        }
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<Node>) -> bool {
        true
    }

    fn show_graph_menu(
        &mut self,
        pos: egui::Pos2,
        ui: &mut Ui,
        _scale: f32,
        snarl: &mut Snarl<Node>,
    ) {
        ui.label("Add node");
        if ui.button("Expr").clicked() {
            snarl.insert_node(pos, Node::ExprNode(ExprNode::new()));
            ui.close_menu();
        }
        if ui.button("String").clicked() {
            snarl.insert_node(pos, Node::Named("".to_owned()));
            ui.close_menu();
        }
    }

    fn has_dropped_wire_menu(&mut self, _src_pins: AnyPins, _snarl: &mut Snarl<Node>) -> bool {
        true
    }

    fn show_dropped_wire_menu(
        &mut self,
        pos: egui::Pos2,
        ui: &mut Ui,
        _scale: f32,
        src_pins: AnyPins,
        snarl: &mut Snarl<Node>,
    ) {
        // In this demo, we create a context-aware node graph menu, and connect a wire
        // dropped on the fly based on user input to a new node created.
        //
        // In your implementation, you may want to define specifications for each node's
        // pin inputs and outputs and compatibility to make this easier.

        ui.label("Add node");

        type PinCompat = usize;
        const PIN_NUM: PinCompat = 1;
        const PIN_STR: PinCompat = 2;

        fn pin_out_compat(node: &Node) -> PinCompat {
            match node {
                Node::Named(_) => PIN_STR,
                Node::ExprNode(_) => PIN_NUM,
            }
        }

        fn pin_in_compat(node: &Node, pin: usize) -> PinCompat {
            match node {
                Node::Named(_) => 0,
                Node::ExprNode(_) => {
                    if pin == 0 {
                        PIN_STR
                    } else {
                        PIN_NUM
                    }
                }
            }
        }

        match src_pins {
            AnyPins::Out(src_pins) => {
                assert!(
                    src_pins.len() == 1,
                    "There's no concept of multi-input nodes in this demo"
                );

                let src_pin = src_pins[0];
                let src_out_ty = pin_out_compat(snarl.get_node(src_pin.node).unwrap());
                let dst_in_candidates = [("Expr", || Node::ExprNode(ExprNode::new()), PIN_STR)];

                for (name, ctor, in_ty) in dst_in_candidates {
                    if src_out_ty & in_ty != 0 && ui.button(name).clicked() {
                        // Create new node.
                        let new_node = snarl.insert_node(pos, ctor());
                        let dst_pin = InPinId {
                            node: new_node,
                            input: 0,
                        };

                        // Connect the wire.
                        snarl.connect(src_pin, dst_pin);
                        ui.close_menu();
                    }
                }
            }
            AnyPins::In(pins) => {
                let all_src_types = pins.iter().fold(0, |acc, pin| {
                    acc | pin_in_compat(snarl.get_node(pin.node).unwrap(), pin.input)
                });

                let dst_out_candidates = [
                    (
                        "String",
                        (|| Node::Named("".to_owned())) as fn() -> Node,
                        PIN_STR,
                    ),
                    ("Expr", || Node::ExprNode(ExprNode::new()), PIN_NUM),
                ];

                for (name, ctor, out_ty) in dst_out_candidates {
                    if all_src_types & out_ty != 0 && ui.button(name).clicked() {
                        // Create new node.
                        let new_node = ctor();
                        let dst_ty = pin_out_compat(&new_node);

                        let new_node = snarl.insert_node(pos, new_node);
                        let dst_pin = OutPinId {
                            node: new_node,
                            output: 0,
                        };

                        // Connect the wire.
                        for src_pin in pins {
                            let src_ty =
                                pin_in_compat(snarl.get_node(src_pin.node).unwrap(), src_pin.input);
                            if src_ty & dst_ty != 0 {
                                // In this demo, input pin MUST be unique ...
                                // Therefore here we drop inputs of source input pin.
                                snarl.drop_inputs(*src_pin);
                                snarl.connect(dst_pin, *src_pin);
                                ui.close_menu();
                            }
                        }
                    }
                }
            }
        };
    }

    fn has_node_menu(&mut self, _node: &Node) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        _scale: f32,
        snarl: &mut Snarl<Node>,
    ) {
        ui.label("Node menu");
        if ui.button("Remove").clicked() {
            snarl.remove_node(node);
            ui.close_menu();
        }
    }

    fn has_on_hover_popup(&mut self, _: &Node) -> bool {
        true
    }

    fn show_on_hover_popup(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        _scale: f32,
        snarl: &mut Snarl<Node>,
    ) {
        match snarl[node] {
            Node::Named(_) => {
                ui.label("Outputs string value");
            }
            Node::ExprNode(_) => {
                ui.label("Evaluates algebraic expression with input for each unique variable name");
            }
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct ExprNode {
    text: String,
    bindings: Vec<String>,
    values: Vec<f64>,
    expr: Expr,
}

impl ExprNode {
    fn new() -> Self {
        ExprNode {
            text: "0".to_string(),
            bindings: Vec::new(),
            values: Vec::new(),
            expr: Expr::Val(0.0),
        }
    }

    fn eval(&self) -> f64 {
        self.expr.eval(&self.bindings, &self.values)
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
enum UnOp {
    Pos,
    Neg,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum Expr {
    Var(String),
    Val(f64),
    UnOp {
        op: UnOp,
        expr: Box<Expr>,
    },
    BinOp {
        lhs: Box<Expr>,
        op: BinOp,
        rhs: Box<Expr>,
    },
}

impl Expr {
    fn eval(&self, bindings: &[String], args: &[f64]) -> f64 {
        let binding_index =
            |name: &str| bindings.iter().position(|binding| binding == name).unwrap();

        match self {
            Expr::Var(ref name) => args[binding_index(name)],
            Expr::Val(value) => *value,
            Expr::UnOp { op, ref expr } => match op {
                UnOp::Pos => expr.eval(bindings, args),
                UnOp::Neg => -expr.eval(bindings, args),
            },
            Expr::BinOp {
                ref lhs,
                op,
                ref rhs,
            } => match op {
                BinOp::Add => lhs.eval(bindings, args) + rhs.eval(bindings, args),
                BinOp::Sub => lhs.eval(bindings, args) - rhs.eval(bindings, args),
                BinOp::Mul => lhs.eval(bindings, args) * rhs.eval(bindings, args),
                BinOp::Div => lhs.eval(bindings, args) / rhs.eval(bindings, args),
            },
        }
    }

    fn extend_bindings(&self, bindings: &mut Vec<String>) {
        match self {
            Expr::Var(name) => {
                if !bindings.contains(name) {
                    bindings.push(name.clone());
                }
            }
            Expr::Val(_) => {}
            Expr::UnOp { expr, .. } => {
                expr.extend_bindings(bindings);
            }
            Expr::BinOp { lhs, rhs, .. } => {
                lhs.extend_bindings(bindings);
                rhs.extend_bindings(bindings);
            }
        }
    }
}

impl syn::parse::Parse for UnOp {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        if lookahead.peek(syn::Token![+]) {
            input.parse::<syn::Token![+]>()?;
            Ok(UnOp::Pos)
        } else if lookahead.peek(syn::Token![-]) {
            input.parse::<syn::Token![-]>()?;
            Ok(UnOp::Neg)
        } else {
            Err(lookahead.error())
        }
    }
}

impl syn::parse::Parse for BinOp {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        if lookahead.peek(syn::Token![+]) {
            input.parse::<syn::Token![+]>()?;
            Ok(BinOp::Add)
        } else if lookahead.peek(syn::Token![-]) {
            input.parse::<syn::Token![-]>()?;
            Ok(BinOp::Sub)
        } else if lookahead.peek(syn::Token![*]) {
            input.parse::<syn::Token![*]>()?;
            Ok(BinOp::Mul)
        } else if lookahead.peek(syn::Token![/]) {
            input.parse::<syn::Token![/]>()?;
            Ok(BinOp::Div)
        } else {
            Err(lookahead.error())
        }
    }
}

impl syn::parse::Parse for Expr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let lookahead = input.lookahead1();

        let lhs;
        if lookahead.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);
            let expr = content.parse::<Expr>()?;
            if input.is_empty() {
                return Ok(expr);
            }
            lhs = expr;
            // } else if lookahead.peek(syn::LitFloat) {
            //     let lit = input.parse::<syn::LitFloat>()?;
            //     let value = lit.base10_parse::<f64>()?;
            //     let expr = Expr::Val(value);
            //     if input.is_empty() {
            //         return Ok(expr);
            //     }
            //     lhs = expr;
        } else if lookahead.peek(syn::LitInt) {
            let lit = input.parse::<syn::LitInt>()?;
            let value = lit.base10_parse::<f64>()?;
            let expr = Expr::Val(value);
            if input.is_empty() {
                return Ok(expr);
            }
            lhs = expr;
        } else if lookahead.peek(syn::Ident) {
            let ident = input.parse::<syn::Ident>()?;
            let expr = Expr::Var(ident.to_string());
            if input.is_empty() {
                return Ok(expr);
            }
            lhs = expr;
        } else {
            let unop = input.parse::<UnOp>()?;

            return Self::parse_with_unop(unop, input);
        }

        let binop = input.parse::<BinOp>()?;

        Self::parse_binop(Box::new(lhs), binop, input)
    }
}

impl Expr {
    fn parse_with_unop(op: UnOp, input: syn::parse::ParseStream) -> syn::Result<Self> {
        let lookahead = input.lookahead1();

        let lhs;
        if lookahead.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);
            let expr = Expr::UnOp {
                op,
                expr: Box::new(content.parse::<Expr>()?),
            };
            if input.is_empty() {
                return Ok(expr);
            }
            lhs = expr;
        } else if lookahead.peek(syn::LitFloat) {
            let lit = input.parse::<syn::LitFloat>()?;
            let value = lit.base10_parse::<f64>()?;
            let expr = Expr::UnOp {
                op,
                expr: Box::new(Expr::Val(value)),
            };
            if input.is_empty() {
                return Ok(expr);
            }
            lhs = expr;
        } else if lookahead.peek(syn::LitInt) {
            let lit = input.parse::<syn::LitInt>()?;
            let value = lit.base10_parse::<f64>()?;
            let expr = Expr::UnOp {
                op,
                expr: Box::new(Expr::Val(value)),
            };
            if input.is_empty() {
                return Ok(expr);
            }
            lhs = expr;
        } else if lookahead.peek(syn::Ident) {
            let ident = input.parse::<syn::Ident>()?;
            let expr = Expr::UnOp {
                op,
                expr: Box::new(Expr::Var(ident.to_string())),
            };
            if input.is_empty() {
                return Ok(expr);
            }
            lhs = expr;
        } else {
            return Err(lookahead.error());
        }

        let op = input.parse::<BinOp>()?;

        Self::parse_binop(Box::new(lhs), op, input)
    }

    fn parse_binop(lhs: Box<Expr>, op: BinOp, input: syn::parse::ParseStream) -> syn::Result<Self> {
        let lookahead = input.lookahead1();

        let rhs;
        if lookahead.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);
            rhs = Box::new(content.parse::<Expr>()?);
            if input.is_empty() {
                return Ok(Expr::BinOp { lhs, op, rhs });
            }
        } else if lookahead.peek(syn::LitFloat) {
            let lit = input.parse::<syn::LitFloat>()?;
            let value = lit.base10_parse::<f64>()?;
            rhs = Box::new(Expr::Val(value));
            if input.is_empty() {
                return Ok(Expr::BinOp { lhs, op, rhs });
            }
        } else if lookahead.peek(syn::LitInt) {
            let lit = input.parse::<syn::LitInt>()?;
            let value = lit.base10_parse::<f64>()?;
            rhs = Box::new(Expr::Val(value));
            if input.is_empty() {
                return Ok(Expr::BinOp { lhs, op, rhs });
            }
        } else if lookahead.peek(syn::Ident) {
            let ident = input.parse::<syn::Ident>()?;
            rhs = Box::new(Expr::Var(ident.to_string()));
            if input.is_empty() {
                return Ok(Expr::BinOp { lhs, op, rhs });
            }
        } else {
            return Err(lookahead.error());
        }

        let next_op = input.parse::<BinOp>()?;

        match (op, next_op) {
            (BinOp::Add | BinOp::Sub, BinOp::Mul | BinOp::Div) => {
                let rhs = Self::parse_binop(rhs, next_op, input)?;
                Ok(Expr::BinOp {
                    lhs,
                    op,
                    rhs: Box::new(rhs),
                })
            }
            _ => {
                let lhs = Expr::BinOp { lhs, op, rhs };
                Self::parse_binop(Box::new(lhs), next_op, input)
            }
        }
    }
}

struct State {
    pos: egui::Pos2,
    graph_str: String,
}

const GRAPH: &str = r#"
digraph G {
  // Nodes
  A ;
  B [label="Node Beee"];
  C [label="Node C"];
  D [label="Node D"];
  E [label="Node E"];

  // Edges (connections)
  A -> B;
  A -> C;
  B -> D;
  C -> D;
  D -> E;
  B -> E;
}
"#;

static LOOP_NUM: AtomicUsize = AtomicUsize::new(0);

impl Default for State {
    fn default() -> Self {
        Self {
            pos: Default::default(),
            graph_str: GRAPH.to_string(),
        }
    }
}
pub struct DemoApp {
    snarl: Snarl<Node>,
    style: SnarlStyle,
    snarl_ui_id: Option<Id>,
    state: State,
}

impl DemoApp {
    pub fn new(cx: &CreationContext) -> Self {
        egui_extras::install_image_loaders(&cx.egui_ctx);

        cx.egui_ctx.style_mut(|style| style.animation_time *= 10.0);

        let snarl = match cx.storage {
            None => Snarl::new(),
            Some(storage) => storage
                .get_string("snarl")
                .and_then(|snarl| serde_json::from_str(&snarl).ok())
                .unwrap_or_else(Snarl::new),
        };
        // let snarl = Snarl::new();

        let style = match cx.storage {
            None => SnarlStyle::new(),
            Some(storage) => storage
                .get_string("style")
                .and_then(|style| serde_json::from_str(&style).ok())
                .unwrap_or_else(SnarlStyle::new),
        };
        // let style = SnarlStyle::new();

        DemoApp {
            snarl,
            style,
            snarl_ui_id: None,
            state: Default::default(),
        }
    }
}

impl App for DemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            // The top panel is often a good place for a menu bar:

            egui::menu::bar(ui, |ui| {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ui.menu_button("File", |ui| {
                        if ui.button("Quit").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close)
                        }
                    });
                    ui.add_space(16.0);
                }

                egui::widgets::global_dark_light_mode_switch(ui);

                if ui.button("Clear All").clicked() {
                    self.snarl = Default::default();
                }
            });
        });

        egui::SidePanel::left("style").show(ctx, |ui| {
            if LOOP_NUM.load(Ordering::SeqCst) == 0 {
                let _ = parse_dot(&mut self.snarl, &self.state.graph_str);
                LOOP_NUM.fetch_add(1, Ordering::SeqCst);
            }
            ui.add(egui::text_edit::TextEdit::multiline(
                &mut self.state.graph_str,
            ));
            if ui.add(egui::Button::new("Parse graph")).clicked() {
                let _ = parse_dot(&mut self.snarl, &self.state.graph_str);
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui_probe::Probe::new(&mut self.style).show(ui);
            });
        });

        if let Some(snarl_ui_id) = self.snarl_ui_id {
            egui::SidePanel::right("selected-list").show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.strong("Selected nodes");

                    let selected =
                        Snarl::<Node>::get_selected_nodes_at("snarl", snarl_ui_id, ui.ctx());
                    let mut selected = selected
                        .into_iter()
                        .map(|id| (id, &self.snarl[id]))
                        .collect::<Vec<_>>();

                    selected.sort_by_key(|(id, _)| *id);

                    let mut remove = None;

                    for (id, node) in selected {
                        ui.horizontal(|ui| {
                            ui.label(format!("{:?}", id));
                            ui.label(node.name());
                            ui.add_space(ui.spacing().item_spacing.x);
                            if ui.button("Remove").clicked() {
                                remove = Some(id);
                            }
                        });
                    }

                    if let Some(id) = remove {
                        self.snarl.remove_node(id);
                    }
                });
            });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            self.snarl_ui_id = Some(ui.id());

            self.snarl.show(&mut DemoViewer, &self.style, "snarl", ui);
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let snarl = serde_json::to_string(&self.snarl).unwrap();
        storage.set_string("snarl", snarl);

        let style = serde_json::to_string(&self.style).unwrap();
        storage.set_string("style", style);
    }
}

// When compiling natively:
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([400.0, 300.0])
            .with_min_inner_size([300.0, 220.0]),
        ..Default::default()
    };

    eframe::run_native(
        "egui-snarl demo",
        native_options,
        Box::new(|cx| Ok(Box::new(DemoApp::new(cx)))),
    )
}

// When compiling to web using trunk:
#[cfg(target_arch = "wasm32")]
fn main() {
    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        eframe::WebRunner::new()
            .start(
                "egui_snarl_demo",
                web_options,
                Box::new(|cx| Ok(Box::new(DemoApp::new(cx)))),
            )
            .await
            .expect("failed to start eframe");
    });
}

fn format_float(v: f64) -> String {
    let v = (v * 1000.0).round() / 1000.0;
    format!("{}", v)
}

fn lower_vg(vg: &mut VisualGraph) {
    vg.to_valid_dag();
    vg.split_text_edges();
    vg.split_long_edges(false);

    for elem in vg.dag.iter() {
        vg.element_mut(elem).resize();
    }
}

fn node_name(e: &Element) -> Result<String> {
    Ok(match &e.shape {
        ShapeKind::Box(x) => x,
        ShapeKind::Circle(x) => x,
        ShapeKind::DoubleCircle(x) => x,
        _ => todo!(),
    }
    .to_string())
}

/// Parse flow
/// g: Graph = DotParser.new(&input).process();
///
fn parse_dot(snarl: &mut Snarl<Node>, input: &str) -> Result<()> {
    let mut parser = DotParser::new(&input);

    let graph = parser.process().map_err(Error::DotParserError)?;
    let mut graph_builder = GraphBuilder::new();
    graph_builder.visit_graph(&graph);
    // has id's as graph.nodes.keys()
    let mut visual_graph = graph_builder.get();

    lower_vg(&mut visual_graph);
    dbg!(&visual_graph);
    Placer::new(&mut visual_graph).layout(false);
    //visual_graph.do_it(false, false, false, &mut svg_writer);
    for nh in visual_graph.iter_nodes() {
        // if not an edge
        if !visual_graph.is_connector(nh) {
            let mid = visual_graph.pos(nh).middle();
            let pos = egui::Pos2 {
                x: mid.x as f32,
                y: mid.y as f32,
            };
            let name = node_name(visual_graph.element(nh))?;
            let node = Node::Named(name);
            snarl.insert_node(pos, node);
        }
    }
    Ok(())
}

#[test]
fn foo() {
    use layout::{
        backends::svg::SVGWriter,
        core::format::Visible,
        gv::{DotParser, GraphBuilder},
        topo::{layout::VisualGraph, placer::place::Placer},
    };

    let contents = r#"
digraph G {
  // Nodes
  A ;
  B [label="Node Beee"];
  C [label="Node C"];
  D [label="Node D"];
  E [label="Node E"];

  // Edges (connections)
  A -> B;
  A -> C;
  B -> D;
  C -> D;
  D -> E;
  B -> E;
}
"#;
    let mut parser = DotParser::new(&contents);

    match parser.process() {
        Ok(g) => {
            let mut gb = GraphBuilder::new();
            gb.visit_graph(&g);
            let mut vg = gb.get();
            //let mut svg_writer = SVGWriter::new();
            lower_vg(&mut vg);
            Placer::new(&mut vg).layout(false);
            //vg.do_it(false, false, false, &mut svg_writer);
            for nh in vg.iter_nodes() {
                dbg!(nh);
                // if not an edge
                if !vg.is_connector(nh) {
                    dbg!(vg.pos(nh));
                }
            }
        }
        Err(err) => {
            parser.print_error();
        }
    }
}
