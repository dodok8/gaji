# 워크플로우 작성

이 가이드는 gaji를 사용하여 타입 안전한 GitHub Actions 워크플로우를 작성하는 방법을 설명합니다.

::: tip 독립 실행형 TypeScript 파일
gaji가 생성한 워크플로우 파일은 완전히 독립적이고 자체 포함됩니다. 어떤 TypeScript 런타임(tsx, ts-node, Deno)으로도 직접 실행하여 워크플로우 JSON을 출력할 수 있습니다. 디버깅과 검사가 쉬워집니다!
:::

## 기본 구조

gaji 워크플로우는 세 가지 주요 구성 요소로 이루어집니다:

1. **액션**: `getAction()`을 사용하여 가져오기
2. **작업**: `Job` 클래스를 사용하여 생성
3. **워크플로우**: `Workflow` 클래스를 사용하여 생성

```ts twoslash
// @filename: workflows/example.ts
// ---cut---
import { getAction, Job, Workflow } from "../generated/index.js";

// 1. 액션 가져오기
const checkout = getAction("actions/checkout@v5");

// 2. 작업 생성
const build = new Job("ubuntu-latest")
  .addStep(checkout({}));

// 3. 워크플로우 생성
const workflow = new Workflow({
  name: "CI",
  on: { push: { branches: ["main"] } },
}).addJob("build", build);

// 4. YAML로 빌드
workflow.build("ci");
```

## 액션 사용하기

### 액션 추가

먼저 액션을 추가하고 타입 생성:

```bash
gaji add actions/checkout@v5
```

### 액션 가져오기

`getAction()`을 사용하여 액션 가져오기:

```typescript
const checkout = getAction("actions/checkout@v5");
const setupNode = getAction("actions/setup-node@v4");
const cache = getAction("actions/cache@v4");
```

### 타입 안전성으로 액션 사용

액션은 설정을 받는 함수를 반환합니다:

```typescript
const step = checkout({
  name: "코드 체크아웃",
  with: {
    // ✅ 모든 입력에 대한 완전한 자동완성!
    repository: "owner/repo",
    ref: "main",
    token: "${{ secrets.GITHUB_TOKEN }}",
    "fetch-depth": 0,
  },
});
```

에디터가 제공하는 것:
- ✅ 모든 액션 입력에 대한 자동완성
- ✅ 타입 체크
- ✅ action.yml의 문서
- ✅ 기본값 표시

## CompositeJob

`CompositeJob`을 사용하여 재사용 가능한 작업 템플릿을 만듭니다:

```ts twoslash
// @noErrors
// @filename: workflows/example.ts
// ---cut---
import { CompositeJob, getAction } from "../generated/index.js";

const checkout = getAction("actions/checkout@v5");
const setupNode = getAction("actions/setup-node@v4");

// 재사용 가능한 작업 클래스 정의
class NodeTestJob extends CompositeJob {
  constructor(nodeVersion: string) {
    super("ubuntu-latest");

    this
      .addStep(checkout({ name: "코드 체크아웃" }))
      .addStep(setupNode({
        name: `Node.js ${nodeVersion} 설정`,
        with: {
          "node-version": nodeVersion,
          cache: "npm",
        },
      }))
      .addStep({ name: "의존성 설치", run: "npm ci" })
      .addStep({ name: "테스트 실행", run: "npm test" });
  }
}

// 워크플로우에서 사용
const workflow = new Workflow({
  name: "테스트 매트릭스",
  on: { push: { branches: ["main"] } },
})
  .addJob("test-node-18", new NodeTestJob("18"))
  .addJob("test-node-20", new NodeTestJob("20"))
  .addJob("test-node-22", new NodeTestJob("22"));
```

### 고급 예제: 매개변수화된 작업

```typescript
class DeployJob extends CompositeJob {
  constructor(environment: "staging" | "production") {
    super("ubuntu-latest");

    this
      .env({
        ENVIRONMENT: environment,
        API_URL: environment === "production"
          ? "https://api.example.com"
          : "https://staging.api.example.com",
      })
      .addStep(checkout({}))
      .addStep(setupNode({ with: { "node-version": "20" } }))
      .addStep({
        name: "배포",
        run: `npm run deploy:${environment}`,
        env: {
          DEPLOY_TOKEN: "${{ secrets.DEPLOY_TOKEN }}",
        },
      });
  }
}
```

**장점:**
- 일반적인 작업 패턴 재사용
- 타입 안전한 매개변수
- 유지보수 용이
- 일관된 작업 구조

## 팁

### 1. 감시 모드 사용

개발 중에는 항상 `gaji dev --watch`를 사용하여 새 액션에 대한 타입을 자동으로 생성하세요.

### 2. 생성된 YAML 검토

커밋하기 전에 항상 생성된 YAML을 검토하여 정확성을 확인하세요.

### 3. 타입 안전성

TypeScript의 타입 체크를 활용하세요:

```typescript
// ❌ 타입 오류 - 알 수 없는 속성 키
setupNode({
  with: {
    "node-versoin": "20",  // 키 이름 오타! ❌
  },
});

// ❌ 타입 오류 - 잘못된 타입
setupNode({
  with: {
    "node-version": 20,  // 문자열이어야 함! ❌
  },
});

// ✅ 올바름
setupNode({
  with: {
    "node-version": "20",  // ✅ 올바른 키와 타입
    cache: "npm",
  },
});
```

**참고**: gaji는 속성 키와 타입에 대한 타입 안전성을 제공하지만, 문자열 값(예: `cache: "npn"` vs `cache: "npm"`)은 컴파일 시점에 검증할 수 없습니다. 이러한 오타를 잡으려면 항상 생성된 YAML을 검토하세요.

## 알려진 제한사항

### `getAction()`은 문자열 리터럴만 지원

gaji는 TypeScript 파일을 **실행하지 않고** 정적으로 분석하여 액션 참조를 추출합니다. 따라서 `getAction()`은 문자열 리터럴만 지원합니다:

```typescript
// ✅ 동작 - 문자열 리터럴
const checkout = getAction("actions/checkout@v5");

// ❌ 동작하지 않음 - 변수 참조
const ref = "actions/checkout@v5";
const checkout = getAction(ref);

// ❌ 동작하지 않음 - 템플릿 리터럴
const checkout = getAction(`actions/checkout@v${version}`);

// ❌ 동작하지 않음 - 객체 속성
const checkout = getAction(config.checkoutRef);
```

gaji가 액션 참조를 감지하지 못하면 `action.yml`을 가져오거나 해당 액션의 타입을 생성하지 않습니다. 항상 `owner/repo@version` 문자열을 직접 전달하세요.

### YAML 출력에서의 문자열 이스케이프

gaji는 JavaScript 문자열을 YAML로 변환하므로, JavaScript에서 이미 이스케이프된 문자가 출력에서 이중 이스케이프될 수 있습니다:

```typescript
// TypeScript에서 \n은 줄바꿈 문자
.addStep({ run: "echo \"hello\nworld\"" })
```

JS 문자열에는 실제 줄바꿈이 포함되어 있어 YAML에서 올바르게 처리됩니다. 하지만 YAML 출력에 리터럴 `\n` 문자를 그대로 유지하려면 이중 이스케이프가 필요합니다:

```typescript
// YAML에서 리터럴 \n을 유지하려면 이중 이스케이프
.addStep({ run: "echo hello\\nworld" })
```

**팁**: 여러 줄 명령어에는 이스케이프 시퀀스 대신 템플릿 리터럴을 사용하세요:

```typescript
.addStep({
  run: `
    echo hello
    echo world
  `.trim(),
})
```

## 다음 단계

- [설정](./configuration.md)에 대해 알아보기
- [예제](/ko/examples/simple-ci) 보기
- [API 레퍼런스](/ko/reference/api) 확인
