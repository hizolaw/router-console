import { test, expect } from "@playwright/test";

async function systemAuth(page: import("@playwright/test").Page) {
  let authenticated = false;
  await page.route("**/api/auth", (route) =>
    route.fulfill({
      json: {
        mode: "system",
        authenticated,
        ...(authenticated ? { username: "operator" } : {}),
      },
    }),
  );
  await page.route("**/api/login", async (route) => {
    const body = route.request().postDataJSON();
    if (body.username !== "operator" || body.password !== "fixture-password") {
      await route.fulfill({ status: 401 });
      return;
    }
    authenticated = true;
    await route.fulfill({ json: { ok: true } });
  });
  await page.route("**/api/logout", (route) => {
    authenticated = false;
    return route.fulfill({ json: { ok: true } });
  });
  await page.route("**/api/status", (route) =>
    route.fulfill(
      authenticated
        ? {
            json: {
              title: "System login fixture",
              version: "test",
              generated_at: Math.floor(Date.now() / 1000),
              plugins: [],
            },
          }
        : { status: 401 },
    ),
  );
  return {
    expire: () => {
      authenticated = false;
    },
  };
}

test("system login, refresh and logout preserve only the server session", async ({
  page,
}) => {
  await systemAuth(page);
  await page.goto("/");
  await expect(page.getByText("使用路由器系统账户登录。")).toBeVisible();
  await expect(page.getByLabel("用户名")).toHaveAttribute(
    "autocomplete",
    "username",
  );
  await expect(page.getByLabel("密码", { exact: true })).toHaveAttribute(
    "autocomplete",
    "current-password",
  );
  await page.getByLabel("用户名").fill("operator");
  await page.getByLabel("密码", { exact: true }).fill("incorrect");
  await page.getByRole("button", { name: "登录控制台" }).click();
  await expect(page.getByRole("alert")).toContainText("用户名或密码不正确");
  await expect(page.getByLabel("密码", { exact: true })).toHaveValue("");
  await page.getByLabel("密码", { exact: true }).fill("fixture-password");
  await page.getByRole("button", { name: "登录控制台" }).click();
  await expect(
    page.getByRole("button", { name: "退出", exact: true }),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole("button", { name: "退出", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(() => [localStorage.length, sessionStorage.length]),
  ).toEqual([0, 0]);
  await page.getByRole("button", { name: "退出", exact: true }).click();
  await expect(page.getByLabel("用户名")).toBeVisible();
  await page.reload();
  await expect(page.getByLabel("用户名")).toBeVisible();
});

test("system session expiry clears service data and requests login", async ({
  page,
}) => {
  const session = await systemAuth(page);
  await page.goto("/");
  await page.getByLabel("用户名").fill("operator");
  await page.getByLabel("密码", { exact: true }).fill("fixture-password");
  await page.getByRole("button", { name: "登录控制台" }).click();
  await expect(
    page.getByRole("button", { name: "退出", exact: true }),
  ).toBeVisible();
  session.expire();
  await page.getByRole("button", { name: "刷新状态" }).click();
  await expect(page.getByRole("alert")).toContainText("登录已过期");
  await expect(page.getByLabel("用户名")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "退出", exact: true }),
  ).toHaveCount(0);
});
