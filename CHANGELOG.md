# Versions

## 0.2.0

1. Enhanced Node Modules Boundary Detection

- Added detection for node_modules/ in paths
- Comprehensive list of known React/JS externals (React, Redux, MUI, etc.)
- Path component analysis to catch ../../node_modules/... patterns
- Performance-optimized with early checks for common packages

2. Smart Path Scoring System

- PathScore struct evaluates traversal safety using multiple heuristics:
  - Parent directory traversals (../ count)
  - Node modules detection
  - Path component length (catches infinite loops)
  - Depth from entry point
- Prevents escaping project boundaries

3. Comprehensive Exclude Patterns

- Default excludes for React/JS ecosystem:
  - Build artifacts: dist/, build/, .next/
  - Test files: _.test._, _.spec._, **tests**/
  - Tooling: .storybook/, coverage/, .cache/
- Glob pattern matching with the glob crate
- Component-level directory exclusion for performance
- Configurable via CLI --exclude flag

4. Traversal Health Monitoring

- TraversalStats tracks progress and detects stuck scenarios
- 30-second stuck threshold (configurable)
- Automatic health checks every 100 files
- Progress recording after successful parsing

5. React.lazy() Support

- Extracts dynamic imports from arrow functions
- Handles both expression and block statement bodies
- Supports React.lazy() and standalone lazy() imports
- Works with both () => import() and () => { return import() } patterns

6. Aggressive Canonical Path Caching

- Dedicated LRU cache for canonicalization (2048 entries)
- Separate from file content cache (512 entries)
- Reduces expensive filesystem calls dramatically
- Thread-safe with async/await support

7. Circuit Breaker Pattern

- Prevents cascading failures with dual limits:
  - Max total errors: 1000
  - Max consecutive errors: 50
- Automatic success tracking resets consecutive counter
- Error statistics logged at completion
- Graceful degradation instead of complete failure

8. CLI Integration

- --exclude flag wired to both bundle and graph commands
- Comma-separated pattern support
- Works alongside existing --max-depth and --max-files options
- Applied to traverser configuration seamlessly
