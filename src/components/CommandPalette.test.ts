// @vitest-environment happy-dom
import { mount } from "@vue/test-utils";
import { afterEach, describe, expect, it } from "vitest";
import CommandPalette from "./CommandPalette.vue";

const commands = [
  { id: "open-directory" as const, label: "Open Directory", enabled: true },
  { id: "new-terminal" as const, label: "New Terminal", enabled: true },
  { id: "open-zed" as const, label: "Open in Zed", enabled: false, disabledReason: "Zed unavailable" },
];

describe("CommandPalette keyboard and filtering", () => {
  let wrapper: ReturnType<typeof mount> | undefined;

  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
  });

  it("opens with Command-K, filters actions, and activates the matching command", async () => {
    wrapper = mount(CommandPalette, { props: { commands } });
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", metaKey: true, bubbles: true }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);

    await wrapper.get('input[aria-label="Filter commands"]').setValue("terminal");
    expect(wrapper.findAll('[role="option"]')).toHaveLength(1);
    expect(wrapper.get('[role="option"]').text()).toContain("New Terminal");
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await wrapper.vm.$nextTick();
    expect(wrapper.emitted("select")).toEqual([["new-terminal"]]);
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
  });

  it("does not select unavailable editors and closes with Escape", async () => {
    wrapper = mount(CommandPalette, { props: { commands } });
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", ctrlKey: true, bubbles: true }));
    await wrapper.vm.$nextTick();
    await wrapper.get('input[aria-label="Filter commands"]').setValue("zed");
    const zed = wrapper.get('[role="option"]');
    expect(zed.attributes("aria-disabled")).toBe("true");
    await zed.trigger("click");
    expect(wrapper.emitted("select")).toBeUndefined();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await wrapper.vm.$nextTick();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
  });
});
