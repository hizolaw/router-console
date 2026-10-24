import { test, expect } from "@playwright/test";
const token = "test-only-public-fixture-token-12345";

test("authenticates, keeps failed services visible, and separates visibility from enablement", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error" && !message.text().includes("401"))
      errors.push(message.text());
  });
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "连接你的控制台" }),
  ).toBeVisible();
  await page.getByLabel("访问令牌").fill("wrong-token");
  await page.getByRole("button", { name: "连接控制台" }).click();
  await expect(page.getByRole("alert")).toContainText("访问令牌不正确");
  await page.getByLabel("访问令牌").fill(token);
  await page.getByRole("button", { name: "连接控制台" }).click();
  await expect(
    page.getByRole("heading", { name: "系统资源", exact: true }),
  ).toBeVisible();
  const failed = page.locator("article").filter({
    has: page.getByRole("heading", { name: "离线服务", exact: true }),
  });
  await expect(failed.locator(".badge")).toHaveText("异常");
  await expect(
    page.getByRole("heading", { name: "隐藏入口", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "全部服务" }).click();
  await expect(
    page.getByRole("heading", { name: "隐藏入口", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "禁用服务", exact: true }),
  ).toHaveCount(0);
  const link = page.getByRole("link", { name: "打开服务" });
  await expect(link).toHaveAttribute("rel", "noopener noreferrer");
  await expect(link).toHaveAttribute("href", "http://localhost:9090/ui/");
  await page.getByRole("button", { name: "关于控制台" }).click();
  await expect(page.getByRole("heading", { name: "第一版边界" })).toBeVisible();
  await page.getByRole("button", { name: "运行概览" }).click();
  await page.screenshot({ path: "../artifacts/desktop.png", fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(
    page.getByRole("heading", { name: "系统资源", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({ path: "../artifacts/mobile.png", fullPage: true });
  await page.getByRole("button", { name: "退出", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "连接你的控制台" }),
  ).toBeVisible();
  expect(errors).toEqual([]);
});

test("an expired cached success never remains green in the browser", async ({
  page,
}) => {
  const now = Math.floor(Date.now() / 1000);
  await page.route("**/api/status", (route) =>
    route.fulfill({
      json: {
        title: "Stale data test",
        version: "0.1.0",
        generated_at: now,
        plugins: [
          {
            id: "dns",
            name: "过期 DNS",
            kind: "dns",
            visible: true,
            url: null,
            health: "healthy",
            summary: "old success",
            metrics: [],
            checked_at: now - 60,
            stale_after_secs: 15,
          },
        ],
      },
    }),
  );
  await page.goto("/");
  const card = page.locator("article");
  await expect(card.locator(".badge")).toHaveText("未知");
  await expect(card).toContainText("采集数据已过期");
});
