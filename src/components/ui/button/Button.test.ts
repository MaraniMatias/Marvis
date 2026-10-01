// @vitest-environment happy-dom
import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import Button from "./Button.vue";

describe("Button", () => {
  it("paints the variant and the size it was asked for", () => {
    // The variants are the only vocabulary the app has for how loud an action is, so the
    // prop has to land on the class the stylesheet draws, not merely be accepted.
    const classes = mount(Button, { props: { variant: "primary", size: "sm" } }).classes();
    expect(classes).toContain("marvis-button");
    expect(classes).toContain("marvis-button-primary");
    expect(classes).toContain("marvis-button-sm");
  });

  it("falls back to a quiet action rather than an unmarked one", () => {
    const classes = mount(Button).classes();
    expect(classes).toContain("marvis-button-secondary");
    expect(classes).toContain("marvis-button-md");
  });

  it("submits only when it is the submit button, and never answers while disabled", async () => {
    const wrapper = mount(Button, { props: { type: "submit", disabled: true } });
    expect(wrapper.attributes("type")).toBe("submit");
    expect(wrapper.attributes("disabled")).toBeDefined();
    await wrapper.trigger("click");
    expect(wrapper.emitted("click")).toBeUndefined();
  });
});
