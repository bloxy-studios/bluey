/**
 * Type declarations for the platform CLI binaries embedded at compile time via
 * Bun's `import … with { type: "file" }`. Each import resolves to a string
 * path (inside $bunfs when compiled, a node_modules path when not).
 */

declare module "@anthropic-ai/claude-agent-sdk-darwin-arm64/claude" {
  const embeddedPath: string;
  export default embeddedPath;
}

declare module "@anthropic-ai/claude-agent-sdk-darwin-x64/claude" {
  const embeddedPath: string;
  export default embeddedPath;
}
