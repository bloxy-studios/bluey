import { describe, expect, it } from "vitest";

import { answerPlainText, markdownToPlainText } from "@/features/hud/answer-text";

describe("answerPlainText (UX-028)", () => {
  it("copies the title, the body and every section without markdown syntax", () => {
    const text = answerPlainText({
      title: "Tell me about a conflict",
      content: "Lead with **ownership**, then the _result_.",
      sections: [
        { id: "s1", title: "Story used", content: "The **migration** at Acme" },
        { id: "s2", title: "Key point", content: "- Listened first\n- Shipped `v2` on time" },
      ],
    });

    expect(text).toBe(
      [
        "Tell me about a conflict",
        "Lead with ownership, then the result.",
        "Story used\nThe migration at Acme",
        "Key point\n- Listened first\n- Shipped v2 on time",
      ].join("\n\n"),
    );
    expect(text).not.toContain("**");
  });

  it("keeps code as written", () => {
    const text = answerPlainText({
      content: "Use this:\n\n```python\ndef f(**kwargs):\n    return _private\n```",
      sections: [{ id: "c", title: "Code", kind: "code", content: "f(**opts)" }],
    });

    expect(text).toBe("Use this:\n\ndef f(**kwargs):\n    return _private\n\nCode\nf(**opts)");
  });

  it("copies a plain answer unchanged", () => {
    expect(answerPlainText({ content: "The answer" })).toBe("The answer");
  });
});

describe("markdownToPlainText", () => {
  it("drops headings, quotes and link syntax but keeps the URL", () => {
    expect(markdownToPlainText("## Plan\n> Quote\nSee [the docs](https://example.com).")).toBe(
      "Plan\nQuote\nSee the docs (https://example.com).",
    );
  });

  it("leaves snake_case identifiers and list bullets alone", () => {
    expect(markdownToPlainText("* use snake_case_name\n* 2 * 3 = 6")).toBe("* use snake_case_name\n* 2 * 3 = 6");
  });
});
