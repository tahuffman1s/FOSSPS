# {name}

A game made with [FOSSPS](https://github.com/tahuffman1s/FOSSPS).

## Play

In this folder, run:

```sh
fossps run
```

Click the window to capture the mouse; Escape frees it.
The window only shows a triangle for now: what the game does is printed in the terminal.

| Action | Keyboard and mouse | Gamepad |
| ------ | ------------------ | ------- |
| Jump   | Space              | Bottom face button |
| Fire   | Left mouse button  | Right trigger |
| Move   | W A S D            | Left stick |

## Change it

The game is `main.luau`. Edit it and save: the running game loads the new version by itself.
If there is a mistake, the terminal shows the line and what to try, and the game keeps running the last version that worked.

## Autocomplete

`fossps.d.luau` describes everything a script can use. In VS Code, install the
[Luau Language Server](https://marketplace.visualstudio.com/items?itemName=JohnnyMorganz.luau-lsp)
extension and open this folder: `.vscode/settings.json` already points it at that file.
`fossps.d.luau` belongs to the engine: `fossps run` rewrites it when FOSSPS is updated, so don't edit it.
