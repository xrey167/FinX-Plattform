{% macro business_day_only(date_expr) -%}
{{ adapter.dispatch('business_day_only', 'finx_finance')(date_expr) }}
{%- endmacro %}

{% macro default__business_day_only(date_expr) -%}
extract(isodow from cast({{ date_expr }} as date)) between 1 and 5
{%- endmacro %}

{% macro postgres__business_day_only(date_expr) -%}
extract(isodow from cast({{ date_expr }} as date)) between 1 and 5
{%- endmacro %}

{% macro clickhouse__business_day_only(date_expr) -%}
toDayOfWeek(toDate({{ date_expr }})) between 1 and 5
{%- endmacro %}
