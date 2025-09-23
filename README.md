# Packlet

A high-performance tool that bundles local code dependencies into a single markdown file by following import statements from an entry point.

## What it does

Packlet traverses your codebase starting from any JavaScript or TypeScript file, discovers all locally imported files, and bundles them into a single document. Unlike tools that bundle entire repositories, Packlet only includes files that are actually imported, making it ideal for sharing specific features or debugging dependency chains.

## Installation

```bash
cargo install packlet
```

Or build from source:

```bash
git clone https://github.com/yourusername/packlet
cd packlet
cargo build --release
```

## Usage

Bundle dependencies from an entry point:

```bash
packlet bundle src/index.ts
```

This creates `index.packlet.md` containing the dependency tree and all discovered local files.

### Options

```bash
# Specify output location
packlet bundle src/app.tsx --output bundle.md

# Limit traversal depth
packlet bundle src/index.js --max-depth 3

# Filter by extensions
packlet bundle main.ts --extensions ts,tsx

# Visualize dependency tree only
packlet graph src/index.js
```

## Features

**Fast** - Parallel dependency analysis using async Rust

**Smart** - Understands ES modules, CommonJS, dynamic imports, and TypeScript paths

**Local-only** - Excludes node_modules and external packages automatically

**Framework-aware** - Handles React, Vue, Svelte, and Angular patterns

**Configurable** - Control traversal depth, file types, and output format

## Output Format

The generated markdown includes:

1. A visual dependency tree showing the import relationships
2. The complete contents of each discovered file
3. Metadata about when the bundle was created

## Example

Given this structure:

```
src/
  index.ts
  utils/helper.ts
  components/Button.tsx
```

Running `packlet bundle src/index.ts` produces a markdown file with the dependency tree and all three files' contents, properly formatted with syntax highlighting.

## Supported Languages

Currently supports JavaScript and TypeScript with full understanding of:

- ES6 imports/exports
- CommonJS require/module.exports
- Dynamic imports
- TypeScript path mappings
- JSX/TSX files

## Configuration

Create `packlet.toml` in your project root for persistent settings:

```toml
[output]
format = "markdown"

[javascript]
resolution = "typescript"
tsconfig_path = "./tsconfig.json"

[traversal]
max_depth = 50
```

## License

MIT
