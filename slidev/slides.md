---
theme: seriph
layout: cover
title: gaji
class: text-center
transition: slide-left
fonts:
  sans: RidiBatang
  mono: D2Coding
  provider: none
---

<img src="/logo.png" class="mx-auto w-40" />

# gaji

**G**itHub **A**ctions **J**ustified **I**mprovements

---

# GitHub Actions의 구조적 문제

1. YAML은 데이터 표현 언어 / 동작을 표시하는 데는 적합하지 않음.
2. 타입 검사가 없습니다.
3. 로컬에서 재현하기가 힘듭니다.


---

# gaji의 접근 방식


**action.yml → TypeScript 타입 자동 생성**

- GraphQL 자동 코드젠과 같은 발상: 스키마(action.yml)에서 타입을 뽑아낸다
- `getAction("actions/checkout@v5")` 호출만으로 해당 액션의 입출력 타입이 생성됨
- 자동완성, 컴파일 시점 오류 검출, IDE 힌트까지 제공

---

# 사용 워크플로우

gaji 워크플로우는 `getAction()` → `Job` → `Workflow` → `.build()`의 흐름으로 구성됩니다.

```bash
cargo install gaji # npm install -D gaji

gaji init --migrate
gaji dev --watch
gaji build
```

---

# 개밥먹기: gaji 릴리즈 CD

<div v-pre>

```ts
const workflow = new Workflow({
  name: "Release",
  on: {
    push: {
      tags: ["v*"],
    },
  },
  permissions: {
    contents: "read",
  },
})
  .addJob("build", build)
  .addJob("upload-release-assets", uploadReleaseAssets)
  .addJob("publish-npm", publishNpm)
  .addJob("publish-crates", publishCrates);

workflow.build("release");
```

</div>

---

# 한계와 앞으로

### 현재 한계

- **최종 산출물은 여전히 YAML** — GitHub Actions 플랫폼의 제약을 벗어날 수 없음
- **세밀한 타입 미지원** — `"npm" | "yarn" | "pnpm"` 같은 유니온 타입 생성 불가
- **GitHub Actions의 템플릿 검증 불가** — `${{ matrix.target.rust_target }}`은 문자열일 뿐
- **문자열 값 오타** — `cache: "npn"` vs `cache: "npm"`은 잡아내지 못함

### 앞으로

- **1.0** — 플러그인 시스템 도입
- action.yml 자동 타입 생성을 TypeScript에 국한하지 않고 다른 언어로 확장

---
layout: center
class: text-center
---

# 감사합니다

<img src="/qr.svg" class="mx-auto w-40" />

<a href="https://hackers.pub/@gaebalgom">@gaebalgom@hackers.pub</a>
<br />
<a href="https://gaji.gaebalgom.work/">gaji.gaebalgom.work</a>

더 많은 소식을 듣고 싶으면 Hackers' Pub에 가입해주세요.