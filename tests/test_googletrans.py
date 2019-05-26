from googletrans import Translator


def test_googletrans():
    translator = Translator()
    result = translator.translate(u'Hello', src='ENGLISH', dest='French')
    assert result.text == u'Bonjour'
