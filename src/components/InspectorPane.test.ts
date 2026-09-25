// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Checkout } from "../domain/workspace";

const { listCheckoutFiles, readCheckoutFile } = vi.hoisted(() => ({
  listCheckoutFiles: vi.fn(),
  readCheckoutFile: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({ listCheckoutFiles, readCheckoutFile }));

import InspectorPane from "./InspectorPane.vue";

function checkout(id: string, isMissing = false): Checkout {
  return {
    id,
    repoId: `repo:${id}`,
    path: `/${id}`,
    canonicalPath: `/${id}`,
    isPrimary: true,
    changedFiles: 0,
    isMissing,
    sessions: [],
  };
}

describe("InspectorPane", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("clears selection and loads files from the newly active checkout", async () => {
    listCheckoutFiles.mockImplementation(async (checkoutId: string) => ({
      entries: [{ name: `${checkoutId}.txt`, path: `${checkoutId}.txt`, kind: "file" }],
      truncated: false,
    }));
    readCheckoutFile.mockImplementation(async (_checkoutId: string, path: string) => ({
      path,
      content: `contents:${path}`,
    }));
    const wrapper = mount(InspectorPane, { props: { checkout: checkout("checkout:first") } });
    await flushPromises();

    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("contents:checkout:first.txt");

    await wrapper.setProps({ checkout: checkout("checkout:second") });
    await flushPromises();
    expect(wrapper.text()).not.toContain("contents:checkout:first.txt");
    expect(wrapper.text()).toContain("checkout:second.txt");
    expect(wrapper.text()).toContain("Select a file to read it.");
    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    await flushPromises();
    expect(readCheckoutFile).toHaveBeenLastCalledWith("checkout:second", "checkout:second.txt");
    expect(wrapper.text()).toContain("contents:checkout:second.txt");
  });

  it("shows empty, loading, permission, and missing states", async () => {
    listCheckoutFiles.mockResolvedValueOnce({ entries: [], truncated: false });
    const wrapper = mount(InspectorPane, { props: { checkout: checkout("empty") } });
    expect(wrapper.get('[role="status"]').text()).toContain("Loading files");
    await flushPromises();
    expect(wrapper.text()).toContain("This checkout is empty.");

    listCheckoutFiles.mockRejectedValueOnce({ code: "permission_denied", message: "denied" });
    await wrapper.setProps({ checkout: checkout("denied") });
    await flushPromises();
    expect(wrapper.text()).toContain("Permission denied");

    await wrapper.setProps({ checkout: checkout("gone", true) });
    expect(wrapper.text()).toContain("Checkout is missing.");
  });
});
