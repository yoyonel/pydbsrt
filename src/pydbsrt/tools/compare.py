import dictdiffer

DEFAULT_RELATIVE_TOLERANCE = 1e-6


def compare_containers(
    computed,
    expected,
    tolerance=DEFAULT_RELATIVE_TOLERANCE,
    filter_nones=False,
    **kwargs,
):
    """

    Args:
        computed:
        expected:
        tolerance:
        filter_nones:

    Returns:

    """
    errors = list(dictdiffer.diff(computed, expected, tolerance=tolerance, **kwargs))
    if filter_nones:

        def error_is_none(error):
            if error[0] == "add":
                return error[-1][-1][-1]
            if error[0] == "change":
                return error[-1][-1]
            return False

        errors = [error for error in errors if error_is_none(error)]
    return errors
