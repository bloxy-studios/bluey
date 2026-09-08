/**
 * The research tool contract shared by both agent backends: names,
 * descriptions and zod input shapes. Claude gets them as in-process MCP tools
 * (`tool(name, description, shape, handler)`), Gemini as function declarations
 * (`parametersJsonSchema` derived from the same shapes), so the model sees the
 * identical tool surface whichever backend runs the job.
 */

import { z } from "zod";

import type { ResearchToolName } from "./protocol";

export interface ToolSpec<Shape extends z.ZodRawShape = z.ZodRawShape> {
  description: string;
  shape: Shape;
}

export const TOOL_SPECS = {
  exa_search: {
    description: "Search the public web (Exa). Returns titles, URLs and snippets/summaries.",
    shape: {
      query: z.string().min(1).describe("Public web search query"),
      numResults: z
        .number()
        .int()
        .min(1)
        .max(10)
        .optional()
        .describe("How many results to return (default 8)"),
      startPublishedDate: z
        .string()
        .optional()
        .describe("ISO 8601 date — only results published after this date"),
    },
  },
  firecrawl_scrape: {
    description: "Fetch a web page as clean Markdown (Firecrawl). Use URLs from search results.",
    shape: { url: z.string().min(1).describe("Absolute URL of the page to scrape") },
  },
  document_read: {
    description: "Read one of the local documents explicitly shared with this research job.",
    shape: { documentId: z.string().min(1).describe("Id of an allowed document") },
  },
} satisfies Record<ResearchToolName, ToolSpec>;

/**
 * JSON Schema for a tool's parameters (zod v4 `toJSONSchema`, without the
 * `$schema` keyword the Gemini API rejects).
 */
export function toolParametersJsonSchema(name: ResearchToolName): Record<string, unknown> {
  const schema = z.toJSONSchema(z.object(TOOL_SPECS[name].shape)) as Record<string, unknown>;
  const { $schema: _ignored, ...rest } = schema;
  return rest;
}
