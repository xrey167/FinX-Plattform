{% macro clean_symbol(symbol_expr) -%}
upper(trim({{ symbol_expr }}))
{%- endmacro %}
