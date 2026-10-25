import { test, expect } from "@playwright/test";

test("optional rules plugin edits rules and subscriptions with revision protection", async ({
  page,
}) => {
  let document = {
    revision: "v1",
    rules: "DOMAIN,existing.example,DIRECT\n",
    subscriptions: [
      {
        name: "ads",
        url: "https://example.com/ads.yaml",
        behavior: "domain",
        format: "yaml",
        policy: "REJECT",
        enabled: true,
      },
    ],
    policies: ["DIRECT", "REJECT", "代理"],
    file_path: "/var/lib/mihomo-router/custom.list",
  };
  let saves = 0;
  let conflict = false;
  let reloads = 0;
  await page.route("**/api/auth", (route) =>
    route.fulfill({
      json: { mode: "system", authenticated: true, username: "operator" },
    }),
  );
  await page.route("**/api/status", (route) =>
    route.fulfill({
      json: {
        title: "Rules fixture",
        version: "test",
        generated_at: Math.floor(Date.now() / 1000),
        plugins: [
          {
            id: "rules",
            name: "代理规则",
            kind: "mihomo_rules",
            visible: true,
            url: null,
            health: "healthy",
            summary: "规则可用",
            metrics: [],
            checked_at: Math.floor(Date.now() / 1000),
            stale_after_secs: 60,
          },
        ],
      },
    }),
  );
  await page.route("**/api/plugins/rules/rules", async (route) => {
    if (route.request().method() === "PUT") {
      if (conflict) {
        await route.fulfill({
          status: 409,
          json: { error: "https://secret.example/token-should-not-appear" },
        });
        return;
      }
      const body = route.request().postDataJSON();
      expect(body.revision).toBe(document.revision);
      document = { ...document, ...body, revision: `v${++saves + 1}` };
    }
    await route.fulfill({ json: document });
  });
  await page.route("**/api/plugins/rules/rules/reload", (route) => {
    reloads++;
    return route.fulfill({ json: {} });
  });
  await page.goto("/");
  await page.getByRole("button", { name: "管理规则" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("/var/lib/mihomo-router/custom.list");
  await dialog.getByLabel("匹配项", { exact: true }).fill("example.org");
  await dialog.getByLabel("目标策略", { exact: true }).selectOption("代理");
  await dialog.getByRole("button", { name: "添加规则", exact: true }).click();
  await expect(dialog.getByLabel("原始规则")).toHaveValue(
    /DOMAIN-SUFFIX,example.org,代理/,
  );
  await dialog.getByRole("button", { name: "添加订阅" }).click();
  const subscription = dialog.getByRole("group", {
    name: "规则订阅 2",
    exact: true,
  });
  await subscription.getByLabel("名称", { exact: true }).fill("custom-block");
  await subscription
    .getByLabel("订阅 URL")
    .fill("https://example.org/list.txt");
  await subscription.getByLabel("文件格式").selectOption("text");
  await subscription.getByLabel("启用").uncheck();
  await dialog.getByRole("button", { name: "验证并应用" }).click();
  await expect(dialog.getByRole("status")).toContainText("规则已验证");
  expect(saves).toBe(1);
  expect(document.subscriptions[1].enabled).toBe(false);
  await dialog.getByLabel("原始规则").fill("DOMAIN,changed.example,DIRECT\n");
  page.once("dialog", (event) => event.dismiss());
  await dialog.getByRole("button", { name: "关闭", exact: true }).click();
  await expect(dialog).toBeVisible();
  conflict = true;
  await dialog.getByRole("button", { name: "验证并应用" }).click();
  await expect(dialog.getByRole("alert")).toContainText("其他位置已修改");
  await expect(dialog).not.toContainText("token-should-not-appear");
  page.once("dialog", (event) => event.accept());
  await dialog.getByRole("button", { name: "重新读取", exact: true }).click();
  await expect(dialog.getByRole("status")).toContainText("已重新读取");
  expect(reloads).toBe(1);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await dialog.evaluate(
      (element) => element.scrollWidth <= element.clientWidth,
    ),
  ).toBe(true);
  await dialog.getByRole("button", { name: "关闭", exact: true }).click();
  await expect(dialog).toHaveCount(0);
});
