# Silent-NAS Docker 快速开始

## 🚀 一键部署

```bash
cd docker
docker-compose up -d
```

## 📊 服务端口

| 服务 | HTTP | WebDAV | S3 | gRPC |
|------|------|--------|-----|------|
| Node1 | 8080 | 8081 | 9001 | 9000 |
| Node2 | 8090 | 8091 | 9011 | 9010 |
| Node3 | 8100 | 8101 | 9021 | 9020 |

## 🧪 测试

```bash
# 上传文件
curl -X POST -F "file=@test.txt" http://localhost:8080/api/files/upload

# 查询文件
curl http://localhost:8090/api/files/list
```
