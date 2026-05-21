{% macro winsorize(col_expr, p) -%}
{{ adapter.dispatch('winsorize', 'finx_finance')(col_expr, p) }}
{%- endmacro %}

{% macro default__winsorize(col_expr, p) -%}
least(
  greatest({{ col_expr }}, percentile_cont({{ p }}) within group (order by {{ col_expr }})),
  percentile_cont(1 - {{ p }}) within group (order by {{ col_expr }})
)
{%- endmacro %}

{% macro postgres__winsorize(col_expr, p) -%}
least(
  greatest({{ col_expr }}, percentile_cont({{ p }}) within group (order by {{ col_expr }})),
  percentile_cont(1 - {{ p }}) within group (order by {{ col_expr }})
)
{%- endmacro %}

{% macro clickhouse__winsorize(col_expr, p) -%}
least(
  greatest({{ col_expr }}, quantileExact({{ p }})({{ col_expr }})),
  quantileExact(1 - {{ p }})({{ col_expr }})
)
{%- endmacro %}
