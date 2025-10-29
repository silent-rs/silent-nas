#!/usr/bin/env bash
set -euo pipefail

# 三节点端到端同步压测脚本 (Docker 版本)
# 功能：
#  - 使用 Docker Compose 启动 3 个节点 + NATS
#  - 通过 WebDAV 向节点1 并发写入 N 个文件
#  - 测量在节点2/3 收敛的端到端时延
#  - 输出成功率、P50/P90/P95/Max（毫秒）
#  - 测试完成后自动清理容器和卷
#
# 依赖：docker、docker-compose、curl、python3
#
# 可配置参数（环境变量）：
#  - N_FILES: 初始写入文件数（默认 100）
#  - PAYLOAD_SIZE: 单文件大小（字节，默认 32KB）
#  - CONCURRENCY: 并发度（默认 10）
#  - SYNC_TIMEOUT_S: 单文件收敛最大等待秒（默认 30）
#  - BENCH_TARGET_P95_MS: 目标 P95 阈值（默认 8000ms）
#

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
DOCKER_DIR="$ROOT_DIR/docker"
WORK_DIR="$ROOT_DIR/scripts/.bench3nodes_docker"
mkdir -p "$WORK_DIR"

N_FILES=${N_FILES:-200}
PAYLOAD_SIZE=${PAYLOAD_SIZE:-32768}
CONCURRENCY=${CONCURRENCY:-10}
SYNC_TIMEOUT_S=${SYNC_TIMEOUT_S:-30}
BENCH_TARGET_P95_MS=${BENCH_TARGET_P95_MS:-8000}

# Docker 端口映射
WEBDAV1=8081
WEBDAV2=8091
WEBDAV3=8101
HTTP1=8080
HTTP2=8090
HTTP3=8100

log() { echo "[bench-docker] $*"; }

# 选择 docker compose 命令（兼容 v1/v2）
if command -v docker-compose >/dev/null 2>&1; then
  DOCKER_COMPOSE=(docker-compose)
else
  DOCKER_COMPOSE=(docker compose)
fi

COMPOSE_FILE="$DOCKER_DIR/docker-compose.yml"
dc() {
  (cd "$DOCKER_DIR" && "${DOCKER_COMPOSE[@]}" -f "$COMPOSE_FILE" "$@")
}

cleanup() {
  log "清理容器和数据..."
  dc down -v --remove-orphans 2>/dev/null || true
  dc rm -f -v 2>/dev/null || true

  # 清理数据目录
  rm -rf "$DOCKER_DIR/data/node"{1,2,3} 2>/dev/null || true
  mkdir -p "$DOCKER_DIR/data/node"{1,2,3}

  # 清理工作目录
  rm -rf "$WORK_DIR" 2>/dev/null || true

  log "清理完成"
}
trap cleanup EXIT

log "构建 Docker 镜像..."
dc build --quiet

log "启动容器（NATS + 3个节点）..."
dc up -d

log "等待容器启动..."
sleep 5

wait_http() {
  local port=$1
  local name=$2
  log "等待 $name HTTP 就绪 (端口 $port)..."
  for i in {1..60}; do
    if curl -fsS "http://127.0.0.1:$port/api/health" >/dev/null 2>&1; then
      log "$name 已就绪"
      return 0
    fi
    sleep 1
  done
  log "❌ $name 超时未就绪"
  dc logs "$name" | tail -50
  return 1
}

wait_http $HTTP1 "node1" || exit 1
wait_http $HTTP2 "node2" || exit 1
wait_http $HTTP3 "node3" || exit 1

log "等待节点发现和连接..."
sleep 10

# 检查节点连接状态
log "检查节点连接状态..."
for node in node1 node2 node3; do
  log "$node 日志片段:"
  dc logs --tail=20 "$node" 2>/dev/null | grep -E "节点|连接|同步|gRPC" | tail -5 || true
done

# 生成 payload
cd "$ROOT_DIR"
mkdir -p "$WORK_DIR"
PAYLOAD="$WORK_DIR/payload.bin"
python3 - <<PY
with open("$PAYLOAD","wb") as f:
    f.write(b'A' * int($PAYLOAD_SIZE))
PY

ms_now() {
  python3 - <<'PY'
import time
print(int(time.time()*1000))
PY
}

# 并发控制
running_jobs() {
  jobs -r | wc -l | tr -d ' '
}

wait_for_slot() {
  local max=$1
  local count=$(running_jobs)
  while (( count >= max )); do
    sleep 0.05
    count=$(running_jobs)
  done
}

measure_file() {
  local idx=$1
  local name="/bench/file_${idx}.dat"
  local t0=$(ms_now)

  # 向节点1写入
  if ! curl -fsS -X PUT --data-binary @"$PAYLOAD" "http://127.0.0.1:$WEBDAV1${name}" >/dev/null 2>&1; then
    echo "UPLOAD_FAILED"
    return 1
  fi

  # 等待同步触发
  sleep 3

  # 轮询节点2/3 收敛
  local deadline=$(( $(date +%s) + SYNC_TIMEOUT_S ))
  local ok2=0 ok3=0
  local checks=0

  while (( $(date +%s) < deadline )); do
    http2=$(curl -s -o /dev/null -w "%{http_code}" -I "http://127.0.0.1:$WEBDAV2${name}") || true
    http3=$(curl -s -o /dev/null -w "%{http_code}" -I "http://127.0.0.1:$WEBDAV3${name}") || true
    [[ "$http2" == "200" ]] && ok2=1
    [[ "$http3" == "200" ]] && ok3=1
    checks=$((checks + 1))

    if (( ok2==1 && ok3==1 )); then
      local t1=$(ms_now)
      local latency=$((t1 - t0))
      >&2 echo "[file_${idx}] 同步成功: ${latency}ms (检查:$checks, node2:$http2, node3:$http3)"
      echo "$latency"
      return 0
    fi
    sleep 0.2
  done

  >&2 echo "[file_${idx}] 超时: node2=$http2, node3=$http3, 检查=$checks"
  echo "TIMEOUT"
  return 1
}

log "开始并发写入 $N_FILES 个文件 (payload=${PAYLOAD_SIZE}B, concurrency=${CONCURRENCY})..."

for i in $(seq 1 $N_FILES); do
  wait_for_slot "$CONCURRENCY"
  (
    out=$(measure_file "$i") || true
    if [[ "$out" != "TIMEOUT" && "$out" != "UPLOAD_FAILED" && -n "$out" ]]; then
      echo "$out" >> "$WORK_DIR/latencies_ms.txt"
    else
      echo "TIMEOUT" >> "$WORK_DIR/latencies_ms.txt"
    fi
  ) &
done

log "等待所有测试完成..."
wait
log "测试完成，统计结果..."

# 统计结果
sort -n "$WORK_DIR/latencies_ms.txt" 2>/dev/null | awk 'BEGIN{ok=0;sum=0}
    $1=="TIMEOUT"{next}
    {v=$1+0; ok++; sum+=v; print v > "/dev/stderr"}
    END{ if(ok==0){exit 0} }' 2>"$WORK_DIR/_vals.txt" >/dev/null || true

if [[ -s "$WORK_DIR/_vals.txt" ]]; then
  cnt=$(wc -l < "$WORK_DIR/_vals.txt" | tr -d ' ')
  sum=$(awk '{s+=$1} END{print s+0}' "$WORK_DIR/_vals.txt")
  avg=$(python3 - <<PY
cnt=int("$cnt"); s=float("$sum");
print("%.1f"%(s/cnt if cnt>0 else 0))
PY
)
  p50_idx=$(( (50*cnt)/100 )); [[ $p50_idx -lt 1 ]] && p50_idx=1
  p90_idx=$(( (90*cnt)/100 )); [[ $p90_idx -lt 1 ]] && p90_idx=1
  p95_idx=$(( (95*cnt)/100 )); [[ $p95_idx -lt 1 ]] && p95_idx=1
  p50=$(sed -n "${p50_idx}p" "$WORK_DIR/_vals.txt")
  p90=$(sed -n "${p90_idx}p" "$WORK_DIR/_vals.txt")
  p95=$(sed -n "${p95_idx}p" "$WORK_DIR/_vals.txt")
  max=$(tail -n 1 "$WORK_DIR/_vals.txt")
  ok=$cnt
  loss=$(python3 - <<PY
total=int("$N_FILES"); ok=int("$cnt");
print("%.2f" % ((total-ok)*100.0/total if total>0 else 0))
PY
)
  echo "SUCCESS=$ok TOTAL=$N_FILES LOSS=${loss}% AVG=${avg}ms P50=${p50}ms P90=${p90}ms P95=${p95}ms MAX=${max}ms" | tee "$WORK_DIR/report.txt"
else
  echo "SUCCESS=0 TOTAL=$N_FILES LOSS=100%" | tee "$WORK_DIR/report.txt"
  log "没有成功的测试结果"
  exit 1
fi

P95=$(awk '{for(i=1;i<=NF;i++){if($i~/^P95=/){sub("P95=","",$i); sub("ms","",$i); print $i}}}' "$WORK_DIR/report.txt")
LOSS=$(awk '{for(i=1;i<=NF;i++){if($i~/^LOSS=/){sub("LOSS=","",$i); sub("%","",$i); print $i}}}' "$WORK_DIR/report.txt")

log "基准阈值：P95<=${BENCH_TARGET_P95_MS}ms，丢失<=0.1%"
rc=0

if [[ -n "$P95" ]] && (( $(echo "$P95 > $BENCH_TARGET_P95_MS" | bc -l 2>/dev/null || echo 0) )); then
  log "P95 超过阈值: ${P95}ms > ${BENCH_TARGET_P95_MS}ms"
  rc=2
fi

if awk -v loss="$LOSS" 'BEGIN{ exit (loss+0 > 0.1 ? 1 : 0) }'; then
  :
else
  log "丢失率过高: ${LOSS}%"
  rc=3
fi

if [[ "$rc" -eq 0 ]]; then
  log "✅ 压测通过"
else
  log "❌ 压测未通过 (rc=$rc)"
  log "=== 节点日志 ==="
  dc logs --tail=50 node1
  dc logs --tail=50 node2
  dc logs --tail=50 node3
fi

exit "$rc"
