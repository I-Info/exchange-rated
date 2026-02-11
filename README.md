# Rated - AUD/CNY Rate Monitor

一个用 Rust 编写的实时澳元汇率监控和展示系统，具有 WebSocket 实时更新和 SQLite 持久化存储功能。

## 功能特性

- Web 界面展示历史汇率趋势
- WebSocket 实时推送汇率更新
- SQLite 数据库持久化存储
- 智能缓存机制，避免重复请求
- 兴业银行人民币对外币牌价爬取（JSON 接口）

## 兴业银行牌价爬虫（CIB）

该爬虫使用浏览器开发者工具确认的 JSON 接口获取牌价数据：

- 数据接口：`https://personalbank.cib.com.cn/pers/main/pubinfo/ifxQuotationQuery/list`
- 请求方式：GET
- 关键参数：
  - `_search=false`
  - `dataSet.nd=当前毫秒时间戳`
  - `dataSet.rows=80`
  - `dataSet.page=1`
  - `dataSet.sidx=`
  - `dataSet.sord=asc`
- 请求头：`X-Requested-With: XMLHttpRequest`

返回 JSON 的 `rows[].cell` 数组包含如下字段顺序：

1. 货币名称
2. 货币符号
3. 货币单位
4. 现汇买入价格
5. 现汇卖出价格
6. 现钞买入价格
7. 现钞卖出价格
