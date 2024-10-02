# Goals

[Graph](https://en.wikipedia.org/wiki/Graph_(discrete_mathematics)) description languages like [DOT](https://en.wikipedia.org/wiki/DOT_(graph_description_language), often produce output that is **close** to good, but they can be difficult to tweak and style.
WYSIWYG graph editors are great for getting the perfect style, but they make automated graph generation difficult.

The goal of this project is to provide the best of both worlds by allowing users to import a graph via a graph description language, edit it in a WYSIWYG GUI, and export them back to the graph description language.

The first Graph Description Language (GDL) we will support is [DOT](https://en.wikipedia.org/wiki/DOT_(graph_description_language). It is widely used and has several Rust implementations.
We're using [egui](https://github.com/emilk/egui) for it's ease of use and flexibility, it runs both natively and on the web.
I'm using [egui_snarl](https://github.com/zakarumych/egui-snarl), for the node graph drawing library. I choose it over [egui_graphs](https://github.com/blitzarx1/egui_graphs) because the latter seemed more oriented toward large scale 3D data visualizations of graphs.

## TODO for an MVP

For just basic graphs:
- [x] Parse DOT graphs and render them
- [x] Basic editing: move nodes
- [ ] Export to DOT

### TODO after MVP

* Edit arbitrary node style attributes
* Directed and undirected connections
* Add nodes
* Support subgraphs
* Support [Mermaid Diagrams](http://mermaid.js.org/intro/syntax-reference.html)
