# elliott-3d

a tiny 3d modeler with no ui, made to be driven by an AI over MCP. you build stuff with tool calls, then it takes a picture.

```
cargo build --release
```

add the binary to your MCP client as a stdio server and you're done. tools are `add_primitive`, `add_mesh`, `add_extrude`, `add_lathe`, `set_object`, `render` and a few more. ask for a `render` to see what you made.

no gpu, no textures, no booleans. y is up.
