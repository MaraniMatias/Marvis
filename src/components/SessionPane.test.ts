// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";
import type { Checkout } from "../domain/workspace";
import SessionPane from "./SessionPane.vue";

vi.mock("./TerminalSession.vue", async () => {
  const { defineComponent: component, h: createElement } = await import("vue");
  return {
    default: component({
      props: {
        checkoutId: { type: String, required: true },
        active: { type: Boolean, default: false },
      },
      emits: ["created", "closed", "statusChanged", "failed"],
      setup(props) {
        return () => createElement("div", { "data-test": "terminal-session", "data-active": props.active });
      },
    }),
  };
});

const checkout: Checkout = {
  id: "checkout:/work/repo",
  repoId: "repo:/work/repo",
  path: "/work/repo",
  canonicalPath: "/work/repo",
  isPrimary: true,
  changedFiles: 0,
  isMissing: false,
  sessions: [
    {
      id: "session:old",
      type: "shell",
      checkoutId: "checkout:/work/repo",
      name: "zsh",
      createdAt: "now",
      status: "inactive",
    },
  ],
};

describe("SessionPane terminal UI", () => {
  it("selects historical sessions and makes it clear their process was not restored", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: false },
    });

    await wrapper.get('[role="tab"]').trigger("click");
    await flushPromises();

    expect(wrapper.emitted("selectSession")).toEqual([["session:old"]]);
    await wrapper.setProps({ activeSessionId: "session:old" });
    expect(wrapper.text()).toContain("This session is inactive");
    expect(wrapper.text()).toContain("its process was not restored");
  });

  it("starts a separate renderer view for a new shell and refuses a missing checkout", async () => {
    const wrapper = mount(SessionPane, {
      props: { checkout, activeSessionId: null, isOpening: false },
    });

    expect(wrapper.text()).toContain("New terminal");
    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(wrapper.findAll('[data-test="terminal-session"]')).toHaveLength(1);
    expect(wrapper.get('[data-test="terminal-session"]').attributes("data-active")).toBe("true");

    await wrapper.setProps({ checkout: { ...checkout, isMissing: true } });
    expect(wrapper.get("header button").attributes("disabled")).toBeDefined();
  });
});
