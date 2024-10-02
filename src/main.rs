use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
};

use eframe::{App, CreationContext};
use egui::{Id, Ui};
use egui_snarl::{
    ui::{AnyPins, PinInfo, SnarlStyle, SnarlViewer},
    InPin, InPinId, NodeId, OutPin, OutPinId, Snarl,
};
use layout::{
    core::format::Visible,
    gv::{
        parser::ast::{EdgeStmt, Graph, NodeStmt, Stmt},
        DotParser, GraphBuilder,
    },
    std_shapes::shapes::{Element, ShapeKind},
    topo::{layout::VisualGraph, placer::place::Placer},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Error parsing DOT graph: [{0}]")]
    DotParserError(String),
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Node {
    name: String,
}

impl Node {
    fn new(name: &str) -> Self {
        Self { name: name.to_string() }
    }
    fn name(&self) -> &str {
        &self.name
    }
}

struct DemoViewer;

impl SnarlViewer<Node> for DemoViewer {
    #[inline]
    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<Node>) {
        for &remote in &to.remotes {
            snarl.disconnect(remote, to.id);
        }

        snarl.connect(from.id, to.id);
    }

    fn title(&mut self, node: &Node) -> String {
        node.name.to_string()
    }

    fn inputs(&mut self, node: &Node) -> usize {
        2
    }

    fn outputs(&mut self, node: &Node) -> usize {
        2
    }

    fn show_input(
        &mut self,
        _pin: &InPin,
        _ui: &mut Ui,
        _scale: f32,
        _snarl: &mut Snarl<Node>,
    ) -> PinInfo {
        PinInfo::default()
    }

    fn show_output(
        &mut self,
        _pin: &OutPin,
        _ui: &mut Ui,
        _scale: f32,
        _snarl: &mut Snarl<Node>,
    ) -> PinInfo {
        PinInfo::default()
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
        if ui.button("String").clicked() {
            snarl.insert_node(pos, Node::new(""));
            ui.close_menu();
        }
    }

    fn has_dropped_wire_menu(&mut self, _src_pins: AnyPins, _snarl: &mut Snarl<Node>) -> bool {
        false
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
        ui.label(snarl[node].name.clone());
    }
}

struct State {
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
                self.snarl = Default::default();
                let _ = parse_dot(&mut self.snarl, &self.state.graph_str);
                LOOP_NUM.fetch_add(1, Ordering::SeqCst);
            } else {
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

fn node_id_from_label(
    g: &Graph,
    id_or_label: &str,
    node_map: &BTreeMap<String, NodeId>,
) -> Option<NodeId> {
    if let Some(s) = node_map.get(id_or_label) {
        return Some(s.clone());
    }

    for s in g.list.list.iter() {
        let Stmt::Node(NodeStmt { id, list }) = s else {
            continue;
        };
        for att in list.list.iter() {
            if id_or_label == id.name {
                if let Some(node) = node_map.get(&id.name) {
                    return Some(node.clone());
                }
            }
            if att.0 == "label" {
                if att.1 == id_or_label || id.name == id_or_label {
                    if let Some(node) = node_map.get(&id.name) {
                        return Some(node.clone());
                    }
                    if let Some(node) = node_map.get(&att.1) {
                        return Some(node.clone());
                    }
                }
            }
        }
    }
    None
}

/// Parse flow
/// g: Graph = DotParser.new(&input).process();
fn parse_dot(snarl: &mut Snarl<Node>, input: &str) -> Result<()> {
    let mut node_map = BTreeMap::new();
    let mut parser = DotParser::new(&input);

    let graph = parser.process().map_err(Error::DotParserError)?;
    let mut graph_builder = GraphBuilder::new();
    graph_builder.visit_graph(&graph);
    // The following creates the "visual" graph and gives positions to the nodes
    let mut visual_graph = graph_builder.get();
    lower_vg(&mut visual_graph);
    Placer::new(&mut visual_graph).layout(false);

    // get all the positions and insert them as nodes
    for nh in visual_graph.iter_nodes() {
        // if not an edge
        if !visual_graph.is_connector(nh) {
            let mid = visual_graph.pos(nh).middle();
            let pos = egui::Pos2 {
                x: mid.x as f32,
                y: mid.y as f32,
            };
            // this is the "label" i need the node "id"
            let name = node_name(visual_graph.element(nh))?;
            let node = Node::new(&name);
            let snarl_node_id = snarl.insert_node(pos, node);
            // save the snarl node_id by it's 'name' wich is dot's NodeId.name or label attr
            node_map.insert(name, snarl_node_id);
        }
    }
    // get the edges (currently from DOT)
    for g in graph.list.list.iter() {
        let Stmt::Edge(EdgeStmt { from, to, .. }) = g else {
            continue;
        };

        // given a dot id, cehck if it's in the node_map
        let from_node_id = node_id_from_label(&graph, &from.name, &node_map).unwrap();
        // start of edge
        let start = OutPinId {
            node: from_node_id.clone(),
            output: 0,
        };

        // start can connect to multiple ends
        for (dot_id, ..) in to.iter() {
            let Some(snarl_to_node_id) = node_id_from_label(&graph, &dot_id.name, &node_map) else {
                panic!();
            };
            let stop = InPinId {
                node: snarl_to_node_id.clone(),
                input: 0,
            };
            snarl.connect(start, stop);
        }
    }
    Ok(())
}
