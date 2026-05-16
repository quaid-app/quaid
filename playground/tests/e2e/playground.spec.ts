import { test, expect } from "@playwright/test";

test("opens the playground shell", async ({ page }) => {
  await page.goto("/chat");
  await expect(page.getByRole("heading", { name: "Memory Search" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Playground sections" })).toBeVisible();
});

test("conversation extraction scenario", async ({ page }) => {
  test.skip(process.env.QUAID_E2E !== "1", "Set QUAID_E2E=1 with a disposable QUAID_DB to run the full Quaid extraction flow.");

  await page.goto("/conversation");
  await page.getByRole("button", { name: /Add Turn/i }).click();
  await page.getByRole("button", { name: /^Close$/i }).click();
  await page.getByRole("button", { name: /Extract/i }).click();
  await page.getByRole("button", { name: /Ask Memory/i }).click();
  await expect(page.getByText(/coffee/i)).toBeVisible();
});
